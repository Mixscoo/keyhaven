//! Session manager and auto-lock.
//!
//! The session manager owns the single piece of long-lived secret state in the
//! backend: while the vault is unlocked it holds the [`OpenVault`] (the VEK and
//! the decrypted [`VaultModel`]) in memory. Everything else — the master
//! password, recovery key, and derived keys — is zeroized immediately after a
//! key is derived (in the crypto/vault layers) and never reaches this module.
//!
//! ## Task 4.1 — in-memory unlocked session state
//!
//! - The VEK and decrypted model live in memory **only while unlocked**. They
//!   are reachable exclusively through the gated accessors
//!   ([`with_vault`](SessionManager::with_vault) /
//!   [`with_vault_mut`](SessionManager::with_vault_mut)), which return
//!   [`KeyhavenError::Locked`] when the vault is locked. Entry commands (task 6)
//!   go through these accessors, so they are automatically gated (Req 3.4).
//! - [`is_unlocked`](SessionManager::is_unlocked) reports the current state.
//! - [`lock`](SessionManager::lock) (the backend of the `lock_vault` command)
//!   drops and best-effort **zeroizes** the decrypted model and VEK, then emits
//!   a `vault-locked` event so the frontend routes back to the Unlock screen
//!   (Req 3.5, 15.5). The VEK is held in `Zeroizing` so it is wiped on drop;
//!   the model's field values are explicitly zeroized before the model is
//!   dropped.
//!
//! ## Task 4.2 — backend auto-lock timer
//!
//! On top of the unlocked-state machine, the session manager owns an
//! authoritative inactivity deadline (see [`auto_lock`]):
//!
//! - On unlock ([`set_unlocked`](SessionManager::set_unlocked)) the deadline is
//!   **armed** from the vault's [`auto_lock_seconds`](crate::model::Settings::auto_lock_seconds)
//!   (`0` = disabled, Req 4.3).
//! - The frontend reports debounced activity through the `report_activity`
//!   command → [`report_activity`](SessionManager::report_activity), which
//!   resets the countdown (Req 4.4).
//! - A backend timer task polls [`check_auto_lock`](SessionManager::check_auto_lock);
//!   on expiry it locks the vault — dropping/zeroizing secrets and emitting the
//!   `vault-locked` event (Req 4.1, 4.2). Keeping the timer in the backend means
//!   a frozen webview cannot bypass it.
//! - Optional lock-on-blur ([`handle_window_blur`](SessionManager::handle_window_blur)),
//!   wired to the Tauri window blur/minimize event, locks when
//!   [`lock_on_blur`](crate::model::Settings::lock_on_blur) is enabled (Req 4.5).
//!
//! The deadline arithmetic lives in [`auto_lock`] behind a [`Clock`] abstraction
//! so it is fully unit-testable without a Tauri runtime or real sleeps.

// Several accessors/constructors here are consumed by later tasks (unlock/create
// commands in task 5, entry commands in task 6) and are not yet referenced from
// the binary while the core is built bottom-up.
#![allow(dead_code)]

pub mod auto_lock;

use std::sync::{Arc, Mutex};

use zeroize::Zeroize;

use crate::error::KeyhavenError;
use crate::model::VaultModel;
use crate::vault::OpenVault;

use auto_lock::{AutoLock, Clock, SystemClock};

/// The event name emitted when the vault transitions to the locked state.
///
/// The frontend listens for this to clear in-memory decrypted data and route
/// back to the Unlock screen (design "Auto-Lock Design").
pub const VAULT_LOCKED_EVENT: &str = "vault-locked";

/// Something that can be notified when the vault locks.
///
/// Abstracting the notification keeps [`SessionManager`] testable without a
/// running Tauri runtime: production code passes a [`tauri::AppHandle`] (which
/// emits the [`VAULT_LOCKED_EVENT`]), while tests pass a lightweight recorder.
pub trait LockNotifier {
    /// Called exactly once each time the vault is locked, after the in-memory
    /// secrets have been zeroized and dropped.
    fn notify_locked(&self);
}

impl LockNotifier for tauri::AppHandle {
    fn notify_locked(&self) {
        use tauri::Emitter;
        // A failure to emit (e.g. during shutdown) must not prevent locking;
        // the security-critical work (zeroize + drop) has already happened.
        let _ = self.emit(VAULT_LOCKED_EVENT, ());
    }
}

/// The interval at which the background timer task polls the inactivity
/// deadline. One second is fine-grained enough for an auto-lock measured in
/// minutes while keeping the idle wakeups negligible.
pub const AUTO_LOCK_TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// Holds the unlocked session state behind interior mutability so it can be
/// shared as Tauri managed state (`State<SessionManager>`) across command
/// invocations.
///
/// `None` means **locked**; `Some(open_vault)` means **unlocked**.
pub struct SessionManager {
    inner: Mutex<Option<OpenVault>>,
    /// Authoritative inactivity deadline / auto-lock timer state (task 4.2).
    auto_lock: AutoLock,
}

impl SessionManager {
    /// Create a new, **locked** session manager (no vault open yet) using the
    /// real system clock for the auto-lock timer.
    pub fn new() -> Self {
        SessionManager::with_clock(Arc::new(SystemClock::new()))
    }

    /// Create a session manager driven by a custom [`Clock`].
    ///
    /// Used by the auto-lock tests to inject a controllable time source so
    /// deadline/expiry behavior can be exercised deterministically.
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        SessionManager {
            inner: Mutex::new(None),
            auto_lock: AutoLock::new(clock),
        }
    }

    /// Whether the vault is currently unlocked.
    pub fn is_unlocked(&self) -> bool {
        self.lock_inner().is_some()
    }

    /// Install a freshly opened vault as the current unlocked session and arm
    /// the auto-lock deadline from the vault's settings.
    ///
    /// Called by the create/unlock commands (tasks 5.x). If a vault was already
    /// unlocked, its secrets are zeroized before being replaced so no stale
    /// decrypted state lingers.
    pub fn set_unlocked(&self, vault: OpenVault) {
        let timeout = vault.model().settings.auto_lock_seconds;
        let mut guard = self.lock_inner();
        if let Some(previous) = guard.take() {
            zeroize_and_drop(previous);
        }
        *guard = Some(vault);
        drop(guard);
        // Arm the inactivity countdown for the freshly unlocked session.
        self.auto_lock.arm(timeout);
    }

    /// Lock the vault: zeroize and drop the decrypted model and VEK, disarm the
    /// auto-lock deadline, then notify listeners via `notifier` (which emits the
    /// `vault-locked` event in production).
    ///
    /// Idempotent: locking an already-locked vault still fires the notification
    /// so the frontend can reliably route to the Unlock screen. Returns `true`
    /// if a vault was actually unlocked (and therefore zeroized) by this call.
    pub fn lock<N: LockNotifier>(&self, notifier: &N) -> bool {
        let was_unlocked = {
            let mut guard = self.lock_inner();
            match guard.take() {
                Some(vault) => {
                    zeroize_and_drop(vault);
                    true
                }
                None => false,
            }
        };
        // Stop the countdown so a locked session has no live deadline.
        self.auto_lock.disarm();
        // Notify outside the lock to avoid holding the mutex across the callback.
        notifier.notify_locked();
        was_unlocked
    }

    /// Report qualifying user activity, resetting the inactivity countdown
    /// (Req 4.4). This is the backend of the `report_activity` command the
    /// frontend calls (debounced).
    ///
    /// A no-op when the vault is locked, so background activity reports cannot
    /// arm a deadline on a locked session.
    pub fn report_activity(&self) {
        if self.is_unlocked() {
            self.auto_lock.record_activity();
        }
    }

    /// Poll the inactivity deadline and lock the vault if it has expired
    /// (Req 4.1, 4.2). Returns `true` if this call locked the vault.
    ///
    /// This is what the backend timer task calls on each tick. Locking goes
    /// through [`lock`](SessionManager::lock), so it zeroizes secrets and emits
    /// the `vault-locked` event exactly like a manual lock.
    pub fn check_auto_lock<N: LockNotifier>(&self, notifier: &N) -> bool {
        if self.auto_lock.is_expired() && self.is_unlocked() {
            self.lock(notifier)
        } else {
            false
        }
    }

    /// Re-apply the auto-lock timeout from the currently unlocked vault's
    /// settings, restarting the countdown. Called after the user changes the
    /// auto-lock setting (task 8.3) so the new duration takes effect immediately.
    pub fn refresh_auto_lock_from_settings(&self) {
        let guard = self.lock_inner();
        if let Some(vault) = guard.as_ref() {
            let timeout = vault.model().settings.auto_lock_seconds;
            drop(guard);
            self.auto_lock.set_timeout(timeout);
        }
    }

    /// Handle a window blur/minimize event: lock the vault if the unlocked
    /// vault's [`Settings::lock_on_blur`] is enabled (Req 4.5). Returns `true`
    /// if this call locked the vault.
    ///
    /// When the vault is already locked or the setting is off, this is a no-op.
    pub fn handle_window_blur<N: LockNotifier>(&self, notifier: &N) -> bool {
        let should_lock = self
            .with_vault(|v| v.model().settings.lock_on_blur)
            .unwrap_or(false);
        if should_lock {
            self.lock(notifier)
        } else {
            false
        }
    }

    /// Run `f` with shared access to the unlocked vault, or return
    /// [`KeyhavenError::Locked`] if the vault is locked.
    ///
    /// This is the gate every read-only entry command (task 6) goes through, so
    /// none can observe decrypted data while locked (Req 3.4).
    pub fn with_vault<R>(
        &self,
        f: impl FnOnce(&OpenVault) -> R,
    ) -> Result<R, KeyhavenError> {
        let guard = self.lock_inner();
        match guard.as_ref() {
            Some(vault) => Ok(f(vault)),
            None => Err(KeyhavenError::Locked),
        }
    }

    /// Run `f` with mutable access to the unlocked vault, or return
    /// [`KeyhavenError::Locked`] if the vault is locked.
    ///
    /// This is the gate every mutating entry command (task 6) goes through.
    pub fn with_vault_mut<R>(
        &self,
        f: impl FnOnce(&mut OpenVault) -> R,
    ) -> Result<R, KeyhavenError> {
        let mut guard = self.lock_inner();
        match guard.as_mut() {
            Some(vault) => Ok(f(vault)),
            None => Err(KeyhavenError::Locked),
        }
    }

    /// Lock the inner mutex, recovering from a poisoned lock.
    ///
    /// A poisoned mutex means another thread panicked while holding the guard.
    /// The contained state (an `Option<OpenVault>`) is still structurally valid,
    /// so we recover the guard rather than propagate the panic — failing closed
    /// here would strand the user's session.
    fn lock_inner(&self) -> std::sync::MutexGuard<'_, Option<OpenVault>> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        SessionManager::new()
    }
}

/// Best-effort zeroize the decrypted contents of `vault` before it is dropped.
///
/// The VEK is held in `Zeroizing` and is wiped automatically when the
/// [`OpenVault`] drops. The decrypted [`VaultModel`], however, contains
/// credential values as plain `String`s, so we overwrite those byte buffers
/// here before the model is freed (Req 15.5 / Property 10). This is best-effort:
/// it cannot defeat prior copies the allocator or OS may have made.
fn zeroize_and_drop(mut vault: OpenVault) {
    zeroize_model(vault.model_mut());
    drop(vault); // VEK (Zeroizing) is wiped here.
}

/// Overwrite every field value (and other free-text secrets) in `model` in
/// place. After this the model still has its structure but no longer holds any
/// credential plaintext.
fn zeroize_model(model: &mut VaultModel) {
    for entry in &mut model.entries {
        if let Some(title) = entry.title.as_mut() {
            title.zeroize();
        }
        for field in &mut entry.fields {
            field.value.zeroize();
            field.label.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KdfParams;
    use crate::model::{Entry, Field, FieldType, ServiceRef};
    use crate::session::auto_lock::TestClock;
    use crate::vault::create_vault;
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A unique temporary directory for a single test, removed on drop.
    /// Mirrors the helper used by the vault repository tests so we avoid pulling
    /// in an extra dev-dependency just for the session tests.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let suffix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("keyhaven-session-test-{pid}-{n}-{suffix}"));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir { path: dir }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Cheap KDF parameters so tests stay fast while still exercising the real
    /// create/unlock crypto path.
    fn fast_params() -> KdfParams {
        KdfParams {
            m_cost: 512,
            t_cost: 1,
            p_cost: 1,
        }
    }

    /// A test notifier that counts how many times the lock event fired.
    struct RecordingNotifier {
        count: Cell<usize>,
    }

    impl RecordingNotifier {
        fn new() -> Self {
            RecordingNotifier {
                count: Cell::new(0),
            }
        }
    }

    impl LockNotifier for RecordingNotifier {
        fn notify_locked(&self) {
            self.count.set(self.count.get() + 1);
        }
    }

    /// Create a real unlocked vault on a temp path and return its handle.
    fn make_open_vault() -> (TempDir, OpenVault) {
        let dir = TempDir::new();
        let path = dir.path().join("test.khv");
        let (mut vault, _recovery) =
            create_vault(&path, b"correct horse battery staple", false, fast_params())
                .expect("create vault");

        // Populate a couple of entries with secret + non-secret fields.
        let model = vault.model_mut();
        model.entries.push(Entry {
            id: "e1".to_string(),
            service_ref: ServiceRef::Catalog {
                id: "facebook".to_string(),
            },
            title: Some("Personal".to_string()),
            fields: vec![
                Field {
                    id: "f1".to_string(),
                    label: "Email".to_string(),
                    field_type: FieldType::Email,
                    value: "me@example.com".to_string(),
                    secret: false,
                },
                Field {
                    id: "f2".to_string(),
                    label: "Password".to_string(),
                    field_type: FieldType::Password,
                    value: "super-secret-value".to_string(),
                    secret: true,
                },
            ],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        });

        (dir, vault)
    }

    /// Like [`make_open_vault`] but lets the test choose the auto-lock timeout
    /// and lock-on-blur setting baked into the vault model.
    fn make_open_vault_with_settings(
        auto_lock_seconds: u32,
        lock_on_blur: bool,
    ) -> (TempDir, OpenVault) {
        let (dir, mut vault) = make_open_vault();
        let settings = &mut vault.model_mut().settings;
        settings.auto_lock_seconds = auto_lock_seconds;
        settings.lock_on_blur = lock_on_blur;
        (dir, vault)
    }

    #[test]
    fn new_session_is_locked() {
        let session = SessionManager::new();
        assert!(!session.is_unlocked());
    }

    #[test]
    fn set_unlocked_transitions_to_unlocked() {
        let session = SessionManager::new();
        let (_dir, vault) = make_open_vault();

        session.set_unlocked(vault);

        assert!(session.is_unlocked());
    }

    #[test]
    fn lock_transitions_back_to_locked_and_notifies() {
        let session = SessionManager::new();
        let (_dir, vault) = make_open_vault();
        session.set_unlocked(vault);
        let notifier = RecordingNotifier::new();

        let was_unlocked = session.lock(&notifier);

        assert!(was_unlocked, "lock should report it locked an open vault");
        assert!(!session.is_unlocked());
        assert_eq!(notifier.count.get(), 1, "lock must emit exactly one event");
    }

    #[test]
    fn lock_when_already_locked_is_idempotent_but_still_notifies() {
        let session = SessionManager::new();
        let notifier = RecordingNotifier::new();

        let was_unlocked = session.lock(&notifier);

        assert!(!was_unlocked, "nothing was unlocked");
        assert!(!session.is_unlocked());
        // Still notifies so the frontend reliably routes to Unlock.
        assert_eq!(notifier.count.get(), 1);
    }

    #[test]
    fn unlock_lock_unlock_cycle_tracks_state() {
        let session = SessionManager::new();
        let notifier = RecordingNotifier::new();

        assert!(!session.is_unlocked());

        let (_dir1, v1) = make_open_vault();
        session.set_unlocked(v1);
        assert!(session.is_unlocked());

        session.lock(&notifier);
        assert!(!session.is_unlocked());

        let (_dir2, v2) = make_open_vault();
        session.set_unlocked(v2);
        assert!(session.is_unlocked());
    }

    #[test]
    fn entry_access_is_gated_when_locked() {
        let session = SessionManager::new();

        let read = session.with_vault(|v| v.model().entries.len());
        assert_eq!(read, Err(KeyhavenError::Locked));

        let write = session.with_vault_mut(|v| v.model_mut().entries.clear());
        assert_eq!(write, Err(KeyhavenError::Locked));
    }

    #[test]
    fn entry_access_succeeds_when_unlocked() {
        let session = SessionManager::new();
        let (_dir, vault) = make_open_vault();
        session.set_unlocked(vault);

        let count = session
            .with_vault(|v| v.model().entries.len())
            .expect("unlocked: access should succeed");
        assert_eq!(count, 1);
    }

    #[test]
    fn access_is_gated_again_after_lock() {
        let session = SessionManager::new();
        let (_dir, vault) = make_open_vault();
        session.set_unlocked(vault);
        let notifier = RecordingNotifier::new();

        // Accessible while unlocked.
        assert!(session.with_vault(|v| v.model().entries.len()).is_ok());

        session.lock(&notifier);

        // Gated once locked (Req 3.4).
        assert_eq!(
            session.with_vault(|v| v.model().entries.len()),
            Err(KeyhavenError::Locked)
        );
    }

    #[test]
    fn zeroize_model_clears_secret_and_nonsecret_field_values() {
        let (_dir, mut vault) = make_open_vault();

        // Sanity: values present before zeroization.
        assert_eq!(vault.model().entries[0].fields[1].value, "super-secret-value");

        zeroize_model(vault.model_mut());

        for entry in &vault.model().entries {
            assert!(entry.title.as_deref().unwrap_or("").is_empty());
            for field in &entry.fields {
                assert!(
                    field.value.is_empty(),
                    "field value must be wiped after zeroize_model"
                );
                assert!(
                    field.label.is_empty(),
                    "field label must be wiped after zeroize_model"
                );
            }
        }
    }

    #[test]
    fn set_unlocked_replacing_existing_session_zeroizes_previous() {
        // Replacing an active session should not leave the old vault accessible.
        let session = SessionManager::new();
        let (_dir1, v1) = make_open_vault();
        session.set_unlocked(v1);

        let (_dir2, mut v2) = make_open_vault();
        v2.model_mut().entries.clear(); // make v2 distinguishable (0 entries)
        session.set_unlocked(v2);

        let count = session.with_vault(|v| v.model().entries.len()).unwrap();
        assert_eq!(count, 0, "the new session replaced the old one");
    }

    // --- Task 4.2: backend auto-lock timer ---

    #[test]
    fn unlock_arms_deadline_and_expiry_triggers_lock() {
        let clock = Arc::new(TestClock::new());
        let session = SessionManager::with_clock(clock.clone());
        let notifier = RecordingNotifier::new();

        // 5-minute auto-lock baked into the vault settings.
        let (_dir, vault) = make_open_vault_with_settings(300, false);
        session.set_unlocked(vault);
        assert!(session.is_unlocked());

        // Before the deadline a poll is a no-op.
        clock.advance_secs(299);
        assert!(!session.check_auto_lock(&notifier));
        assert!(session.is_unlocked());
        assert_eq!(notifier.count.get(), 0);

        // Crossing the deadline locks the vault and emits exactly one event.
        clock.advance_secs(1);
        let locked = session.check_auto_lock(&notifier);
        assert!(locked, "expiry must trigger a lock");
        assert!(!session.is_unlocked());
        assert_eq!(notifier.count.get(), 1, "lock emits the vault-locked event");

        // Entry access is gated again after the auto-lock (Req 4.2).
        assert_eq!(
            session.with_vault(|v| v.model().entries.len()),
            Err(KeyhavenError::Locked)
        );
    }

    #[test]
    fn reported_activity_resets_the_deadline_and_prevents_lock() {
        let clock = Arc::new(TestClock::new());
        let session = SessionManager::with_clock(clock.clone());
        let notifier = RecordingNotifier::new();

        let (_dir, vault) = make_open_vault_with_settings(300, false);
        session.set_unlocked(vault);

        // Most of the way to the deadline, then the user does something.
        clock.advance_secs(290);
        session.report_activity();

        // 290s after the reset is still inside the fresh window: no lock.
        clock.advance_secs(290);
        assert!(!session.check_auto_lock(&notifier));
        assert!(session.is_unlocked(), "activity must keep the vault unlocked");

        // Past the full timeout measured from the reset: now it locks.
        clock.advance_secs(10);
        assert!(session.check_auto_lock(&notifier));
        assert!(!session.is_unlocked());
        assert_eq!(notifier.count.get(), 1);
    }

    #[test]
    fn auto_lock_disabled_never_expires() {
        let clock = Arc::new(TestClock::new());
        let session = SessionManager::with_clock(clock.clone());
        let notifier = RecordingNotifier::new();

        // auto_lock_seconds == 0 disables auto-lock (Req 4.3).
        let (_dir, vault) = make_open_vault_with_settings(0, false);
        session.set_unlocked(vault);

        clock.advance_secs(86_400); // a full day
        assert!(!session.check_auto_lock(&notifier));
        assert!(session.is_unlocked(), "disabled auto-lock never locks");
        assert_eq!(notifier.count.get(), 0);
    }

    #[test]
    fn report_activity_is_ignored_while_locked() {
        let clock = Arc::new(TestClock::new());
        let session = SessionManager::with_clock(clock.clone());
        let notifier = RecordingNotifier::new();

        // No vault unlocked: reporting activity must not arm a deadline.
        session.report_activity();
        assert!(!session.auto_lock.is_armed());

        clock.advance_secs(10);
        assert!(!session.check_auto_lock(&notifier));
        assert!(!session.is_unlocked());
    }

    #[test]
    fn refresh_auto_lock_applies_updated_timeout() {
        let clock = Arc::new(TestClock::new());
        let session = SessionManager::with_clock(clock.clone());
        let notifier = RecordingNotifier::new();

        let (_dir, vault) = make_open_vault_with_settings(300, false);
        session.set_unlocked(vault);

        // Simulate the user shortening the timeout to 30s via update_settings.
        session
            .with_vault_mut(|v| v.model_mut().settings.auto_lock_seconds = 30)
            .unwrap();
        session.refresh_auto_lock_from_settings();

        clock.advance_secs(29);
        assert!(!session.check_auto_lock(&notifier));
        clock.advance_secs(1);
        assert!(session.check_auto_lock(&notifier), "new 30s timeout applies");
        assert!(!session.is_unlocked());
    }

    #[test]
    fn lock_on_blur_locks_only_when_enabled() {
        let notifier = RecordingNotifier::new();

        // Setting enabled: a blur event locks the vault (Req 4.5).
        let session = SessionManager::new();
        let (_dir, vault) = make_open_vault_with_settings(300, true);
        session.set_unlocked(vault);
        let locked = session.handle_window_blur(&notifier);
        assert!(locked, "blur should lock when lock_on_blur is enabled");
        assert!(!session.is_unlocked());

        // Setting disabled: a blur event is a no-op.
        let session2 = SessionManager::new();
        let (_dir2, vault2) = make_open_vault_with_settings(300, false);
        session2.set_unlocked(vault2);
        let notifier2 = RecordingNotifier::new();
        let locked2 = session2.handle_window_blur(&notifier2);
        assert!(!locked2, "blur should not lock when lock_on_blur is disabled");
        assert!(session2.is_unlocked());
    }

    #[test]
    fn lock_on_blur_when_locked_is_noop() {
        let session = SessionManager::new();
        let notifier = RecordingNotifier::new();
        // Nothing unlocked: blur cannot lock and must not panic.
        assert!(!session.handle_window_blur(&notifier));
    }

    #[test]
    fn manual_lock_disarms_so_later_poll_is_noop() {
        let clock = Arc::new(TestClock::new());
        let session = SessionManager::with_clock(clock.clone());
        let notifier = RecordingNotifier::new();

        let (_dir, vault) = make_open_vault_with_settings(300, false);
        session.set_unlocked(vault);

        // Manual lock disarms the deadline.
        assert!(session.lock(&notifier));
        assert!(!session.auto_lock.is_armed());

        // A subsequent poll well past the original deadline does nothing extra.
        clock.advance_secs(10_000);
        assert!(!session.check_auto_lock(&notifier));
        assert_eq!(notifier.count.get(), 1, "only the manual lock fired");
    }
}
