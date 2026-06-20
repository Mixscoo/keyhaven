//! Backend-authoritative inactivity deadline tracking for auto-lock (task 4.2).
//!
//! The security-critical part of auto-lock is that the *timer lives in the Rust
//! backend*, not the webview: a frozen or compromised frontend cannot keep the
//! vault unlocked past its deadline. The frontend only *reports activity*
//! (debounced) via a command; the backend owns the authoritative deadline and a
//! background task that locks the vault when it expires (design "Auto-Lock
//! Design").
//!
//! This module isolates the pure deadline arithmetic so it is unit-testable
//! without a Tauri runtime *and* without real wall-clock sleeps. Time is read
//! through the [`Clock`] trait: production uses [`SystemClock`] (a monotonic
//! `Instant`), while tests use a [`TestClock`] they can advance deterministically.
//!
//! Configuration comes from [`crate::model::Settings::auto_lock_seconds`]:
//! a value of `0` means **auto-lock disabled** (Req 4.3), in which case no
//! deadline is ever armed and the vault never auto-locks.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A monotonic millisecond time source.
///
/// Abstracted so the deadline logic can be driven by a fake clock in tests. Only
/// relative differences matter, so any monotonically non-decreasing source works.
pub trait Clock: Send + Sync {
    /// Milliseconds elapsed since some fixed, implementation-defined epoch. Must
    /// be monotonically non-decreasing.
    fn now_millis(&self) -> u64;
}

/// Production [`Clock`] backed by a monotonic [`Instant`] captured at startup.
pub struct SystemClock {
    base: Instant,
}

impl SystemClock {
    /// Create a clock whose epoch is "now".
    pub fn new() -> Self {
        SystemClock {
            base: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        self.base.elapsed().as_millis() as u64
    }
}

/// A controllable [`Clock`] for deterministic tests.
///
/// Time only moves when [`advance`](TestClock::advance) is called, so tests can
/// place the "current time" exactly on either side of a deadline.
pub struct TestClock {
    now: AtomicU64,
}

impl TestClock {
    /// Create a test clock starting at `t = 0`.
    pub fn new() -> Self {
        TestClock {
            now: AtomicU64::new(0),
        }
    }

    /// Move the clock forward by `millis`.
    pub fn advance(&self, millis: u64) {
        self.now.fetch_add(millis, Ordering::SeqCst);
    }

    /// Move the clock forward by `secs` seconds.
    pub fn advance_secs(&self, secs: u64) {
        self.advance(secs.saturating_mul(1000));
    }
}

impl Default for TestClock {
    fn default() -> Self {
        TestClock::new()
    }
}

impl Clock for TestClock {
    fn now_millis(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// The current inactivity deadline configuration and target time.
#[derive(Clone, Copy, Debug)]
struct Deadline {
    /// Configured inactivity timeout in seconds; `0` means auto-lock disabled.
    timeout_secs: u32,
    /// Absolute monotonic time (ms) at which the vault should auto-lock, or
    /// `None` when disarmed (locked) or disabled (`timeout_secs == 0`).
    at_millis: Option<u64>,
}

/// Tracks the inactivity deadline behind a mutex so it can be shared across the
/// command thread(s) and the background timer task.
///
/// The deadline is **armed** on unlock and reset on every reported activity; it
/// is **disarmed** on lock. [`is_expired`](AutoLock::is_expired) is what the
/// background timer polls.
pub struct AutoLock {
    clock: Arc<dyn Clock>,
    inner: Mutex<Deadline>,
}

impl AutoLock {
    /// Create a disarmed auto-lock driven by `clock`.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        AutoLock {
            clock,
            inner: Mutex::new(Deadline {
                timeout_secs: 0,
                at_millis: None,
            }),
        }
    }

    /// Lock the inner mutex, recovering from poisoning (the contained state is a
    /// plain `Copy` struct and is always structurally valid).
    fn guard(&self) -> std::sync::MutexGuard<'_, Deadline> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Compute the deadline from "now" given a timeout, or `None` when disabled.
    fn deadline_from_now(&self, timeout_secs: u32) -> Option<u64> {
        if timeout_secs == 0 {
            None
        } else {
            Some(self.clock.now_millis() + (timeout_secs as u64) * 1000)
        }
    }

    /// Arm the deadline with `timeout_secs`, starting the countdown from now.
    ///
    /// Called when the vault is unlocked. A `timeout_secs` of `0` disables
    /// auto-lock (no deadline is set) (Req 4.3).
    pub fn arm(&self, timeout_secs: u32) {
        let at = self.deadline_from_now(timeout_secs);
        let mut g = self.guard();
        g.timeout_secs = timeout_secs;
        g.at_millis = at;
    }

    /// Reset the inactivity countdown using the configured timeout (Req 4.4).
    ///
    /// Called whenever qualifying user activity is reported. If auto-lock is
    /// disabled (`timeout_secs == 0`) this is a no-op.
    pub fn record_activity(&self) {
        let timeout = self.guard().timeout_secs;
        let at = self.deadline_from_now(timeout);
        let mut g = self.guard();
        g.at_millis = at;
    }

    /// Update the configured timeout while keeping the session armed, restarting
    /// the countdown from now. Used when the user changes the auto-lock setting
    /// while the vault is unlocked.
    pub fn set_timeout(&self, timeout_secs: u32) {
        self.arm(timeout_secs);
    }

    /// Disarm the deadline (called on lock). The vault can no longer auto-lock
    /// until it is unlocked and armed again.
    pub fn disarm(&self) {
        let mut g = self.guard();
        g.at_millis = None;
    }

    /// Whether the inactivity deadline has passed.
    ///
    /// Returns `false` when disarmed or when auto-lock is disabled, so a disabled
    /// timeout never triggers a lock.
    pub fn is_expired(&self) -> bool {
        let g = self.guard();
        match g.at_millis {
            Some(at) => self.clock.now_millis() >= at,
            None => false,
        }
    }

    /// The configured timeout in seconds (`0` == disabled). Exposed for tests
    /// and diagnostics.
    pub fn timeout_secs(&self) -> u32 {
        self.guard().timeout_secs
    }

    /// Whether a deadline is currently armed (a countdown is in progress).
    pub fn is_armed(&self) -> bool {
        self.guard().at_millis.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_test_clock() -> (Arc<TestClock>, AutoLock) {
        let clock = Arc::new(TestClock::new());
        let auto = AutoLock::new(clock.clone());
        (clock, auto)
    }

    #[test]
    fn disabled_timeout_never_arms_or_expires() {
        let (clock, auto) = with_test_clock();
        auto.arm(0);
        assert!(!auto.is_armed(), "timeout 0 must not arm a deadline");
        clock.advance_secs(10_000);
        assert!(!auto.is_expired(), "disabled auto-lock never expires");
    }

    #[test]
    fn arm_then_expire_after_timeout() {
        let (clock, auto) = with_test_clock();
        auto.arm(300); // 5 minutes
        assert!(auto.is_armed());
        assert!(!auto.is_expired(), "fresh deadline is not yet expired");

        clock.advance_secs(299);
        assert!(!auto.is_expired(), "one second before deadline: still unlocked");

        clock.advance_secs(1); // exactly at deadline
        assert!(auto.is_expired(), "at the deadline the vault should auto-lock");
    }

    #[test]
    fn record_activity_resets_the_deadline() {
        let (clock, auto) = with_test_clock();
        auto.arm(300);

        // Advance most of the way, then report activity: the countdown restarts.
        clock.advance_secs(290);
        assert!(!auto.is_expired());
        auto.record_activity();

        // 290s after the reset is still within the fresh 300s window.
        clock.advance_secs(290);
        assert!(
            !auto.is_expired(),
            "activity must reset the inactivity timer (Req 4.4)"
        );

        // Crossing the full timeout from the reset point finally expires it.
        clock.advance_secs(10);
        assert!(auto.is_expired());
    }

    #[test]
    fn record_activity_is_noop_when_disabled() {
        let (clock, auto) = with_test_clock();
        auto.arm(0);
        auto.record_activity();
        clock.advance_secs(99_999);
        assert!(!auto.is_expired());
    }

    #[test]
    fn disarm_clears_the_deadline() {
        let (clock, auto) = with_test_clock();
        auto.arm(60);
        assert!(auto.is_armed());
        auto.disarm();
        assert!(!auto.is_armed());
        clock.advance_secs(120);
        assert!(!auto.is_expired(), "a disarmed deadline never expires");
    }

    #[test]
    fn set_timeout_restarts_countdown_with_new_duration() {
        let (clock, auto) = with_test_clock();
        auto.arm(300);
        clock.advance_secs(100);

        // Shorten the timeout to 30s: countdown restarts from now.
        auto.set_timeout(30);
        assert_eq!(auto.timeout_secs(), 30);
        clock.advance_secs(29);
        assert!(!auto.is_expired());
        clock.advance_secs(1);
        assert!(auto.is_expired());
    }
}
