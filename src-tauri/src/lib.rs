//! Keyhaven Rust backend (the trusted core).
//!
//! All cryptographic operations and secret handling live here. The Svelte
//! frontend is a presentation layer that talks to this core over Tauri IPC and
//! never performs encryption or networking.
//!
//! Module layout (skeleton; implemented in later tasks):
//! - [`crypto`]  KDF (Argon2id), AEAD (XChaCha20-Poly1305), random, key wrapping.
//! - [`vault`]   vault file (de)serialization, versioned header, repository.
//! - [`session`] in-memory unlocked state, auto-lock timer, zeroization.
//! - [`catalog`] bundled service catalog loading and search.
//! - [`commands`] Tauri command handlers (the IPC surface).
//! - [`model`]   serde data structures shared across modules.
//! - [`error`]   the shared `KeyhavenError` returned across the IPC boundary.

mod catalog;
mod commands;
mod crypto;
mod entries;
mod error;
mod generator;
mod model;
mod session;
mod vault;

use std::sync::Arc;

use tauri::Manager;

use session::SessionManager;

/// Build and run the Tauri application.
///
/// Networking is intentionally absent: no HTTP plugin is registered. Only the
/// filesystem (dialog-scoped), dialog, and clipboard plugins are enabled, as
/// constrained further by `capabilities/default.json`.
pub fn run() {
    // The session manager is shared as `Arc` so the background auto-lock timer
    // task and the window-event handler can hold their own owned references
    // alongside the Tauri-managed state used by command handlers.
    let session = Arc::new(SessionManager::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(session)
        .setup(|app| {
            // --- Backend-authoritative auto-lock timer (task 4.2, Req 4.1/4.2) ---
            // Poll the inactivity deadline on a fixed tick; on expiry the vault
            // locks (zeroize + `vault-locked` event). Running this in the backend
            // means a frozen webview cannot keep the vault unlocked.
            let timer_session = app.state::<Arc<SessionManager>>().inner().clone();
            let timer_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(session::AUTO_LOCK_TICK);
                timer_session.check_auto_lock(&timer_handle);
            });

            // --- Optional lock-on-blur (task 4.2, Req 4.5) ---
            // When the main window loses focus (covering minimize), lock the
            // vault if the user enabled `lock_on_blur` in settings.
            if let Some(window) = app.get_webview_window("main") {
                let blur_session = app.state::<Arc<SessionManager>>().inner().clone();
                let blur_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        blur_session.handle_window_blur(&blur_handle);
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::vault_exists,
            commands::is_unlocked,
            commands::lock_vault,
            commands::report_activity,
            commands::create_vault,
            commands::unlock_with_password,
            commands::unlock_with_recovery_key,
            commands::change_master_password,
            commands::list_entries,
            commands::get_entry,
            commands::create_entry,
            commands::update_entry,
            commands::delete_entry,
            commands::search_catalog,
            commands::list_custom_services,
            commands::create_custom_service,
            commands::generate_password,
            commands::copy_secret_to_clipboard,
            commands::get_settings,
            commands::update_settings,
            commands::export_vault,
            commands::import_vault,
            commands::import_external_vault,
            commands::save_recovery_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Keyhaven application");
}
