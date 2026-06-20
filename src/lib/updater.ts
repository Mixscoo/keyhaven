/*
 * Auto-update check.
 *
 * On startup the app asks GitHub (via the Tauri updater plugin, which runs in
 * the trusted Rust backend — not the webview) whether a newer signed release is
 * available. If so, it downloads and installs the update and relaunches, so
 * users stay on the latest version without hunting for installers.
 *
 * The update is signed with our private key and verified against the public key
 * baked into the app, so a tampered or unofficial build is rejected.
 *
 * Everything here is best-effort: offline, no update, or running in `tauri dev`
 * (where there's no matching release) all resolve to "nothing to do" silently.
 * The Tauri plugin modules are imported dynamically so this file is harmless to
 * load outside a Tauri runtime (e.g. tests).
 */
import { toast } from "./stores/toast";

export async function checkForUpdates(): Promise<void> {
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) return; // already on the latest version

    toast.push("info", `Updating Keyhaven to ${update.version}…`);
    await update.downloadAndInstall();

    // Relaunch into the freshly installed version.
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  } catch {
    // Offline, no update available, or updater unavailable (e.g. dev build) —
    // never surface this to the user.
  }
}
