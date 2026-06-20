<script lang="ts">
  /*
   * Setup (first-run) screen — Task 10.1.
   *
   * Two-step flow:
   *   1. "create": choose a master password (entered twice), see a live strength
   *      meter, optionally opt in to a recovery key. Mismatched passwords block
   *      creation (Req 1.3); weak passwords warn but do not block (Req 1.4).
   *   2. "reveal": shown ONLY when a recovery key was generated. The key is
   *      displayed exactly once (Req 2.2) with a security warning (Req 2.5) and a
   *      mandatory "I saved it" confirmation gate (Req 2.4) before entering the
   *      vault.
   *
   * All cryptography happens in the Rust backend via `createVault`; this screen
   * only collects input, calls the command, and routes into the unlocked shell.
   * Requirements: 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.4, 2.5, 2.6.
   */
  import { get } from "svelte/store";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import * as api from "../lib/api";
  import type { VaultSummary } from "../lib/types";
  import {
    vaultPath,
    session,
    enterUnlocked,
    resolveDefaultVaultPath,
  } from "../lib/stores/session";
  import { toast } from "../lib/stores/toast";

  // --- Flow state -----------------------------------------------------------
  type Step = "create" | "reveal";
  let step = $state<Step>("create");

  let password = $state("");
  let confirm = $state("");
  let generateRecovery = $state(true);
  let showPassword = $state(false);

  let submitting = $state(false);
  let error = $state("");
  // True while the recovery key is being written to a user-chosen file.
  let savingKey = $state(false);
  // True while importing an existing vault from another device.
  let importingExisting = $state(false);

  // Populated only when a recovery key was generated. Held transiently for the
  // one-time reveal and dropped once the user proceeds into the vault.
  let recoveryKey = $state("");
  let savedConfirmed = $state(false);

  // Carries the freshly created vault summary from create → reveal step so the
  // final "Continue" can hand it to `enterUnlocked`.
  let pendingSummary = $state<VaultSummary | null>(null);

  // --- Password strength (client-side heuristic) ----------------------------
  // Length + character-class variety. Intentionally simple and offline — no
  // network or external dependency (Req 1.4).
  interface Strength {
    score: number; // 0..4
    label: string;
    /** True when the password is weak enough to warrant a (non-blocking) warning. */
    weak: boolean;
  }

  function estimateStrength(pw: string): Strength {
    if (pw.length === 0) {
      return { score: 0, label: "", weak: false };
    }

    const classes =
      (/[a-z]/.test(pw) ? 1 : 0) +
      (/[A-Z]/.test(pw) ? 1 : 0) +
      (/[0-9]/.test(pw) ? 1 : 0) +
      (/[^A-Za-z0-9]/.test(pw) ? 1 : 0);

    let score = 0;
    if (pw.length >= 8) score++;
    if (pw.length >= 12) score++;
    if (pw.length >= 16) score++;
    if (classes >= 2) score++;
    if (classes >= 3) score++;

    // Very short passwords can never score well, regardless of variety.
    if (pw.length < 8) score = Math.min(score, 1);

    score = Math.min(score, 4);

    const labels = ["Very weak", "Weak", "Fair", "Good", "Strong"];
    return { score, label: labels[score], weak: score < 2 };
  }

  const strength = $derived(estimateStrength(password));
  const mismatch = $derived(confirm.length > 0 && password !== confirm);
  const canSubmit = $derived(
    password.length > 0 && confirm.length > 0 && !mismatch && !submitting,
  );

  // --- Actions --------------------------------------------------------------
  async function handleCreate(event: SubmitEvent) {
    event.preventDefault();
    error = "";

    if (password.length === 0) {
      error = "Enter a master password.";
      return;
    }
    if (password !== confirm) {
      // Req 1.3 — do NOT create the vault when the two entries differ.
      error = "The passwords don't match. Please re-enter them.";
      return;
    }

    submitting = true;
    try {
      const path = get(vaultPath);
      const result = await api.createVault(password, generateRecovery, path);

      // A freshly created vault is empty and unlocked. Build its summary.
      const summary: VaultSummary = {
        hasRecovery: generateRecovery,
        entryCount: 0,
        unlockedViaRecovery: false,
      };

      if (generateRecovery && result.recoveryKey) {
        // Gate entry behind the one-time reveal + "I saved it" confirmation.
        recoveryKey = result.recoveryKey;
        pendingSummary = summary;
        // Clear the entered secrets now that the key is derived.
        password = "";
        confirm = "";
        step = "reveal";
      } else {
        // Password-only vault: go straight into the unlocked shell.
        password = "";
        confirm = "";
        await enterUnlocked(summary);
      }
    } catch (e) {
      error = errorMessage(e);
      toast.push("danger", "Could not create the vault.");
    } finally {
      submitting = false;
    }
  }

  async function finishWithRecovery() {
    if (!savedConfirmed || !pendingSummary) return;
    const summary = pendingSummary;
    // Drop the recovery key from memory before leaving the screen.
    recoveryKey = "";
    pendingSummary = null;
    await enterUnlocked(summary);
  }

  async function copyRecoveryKey() {
    try {
      await navigator.clipboard.writeText(recoveryKey);
      toast.push("success", "Recovery key copied to the clipboard.");
    } catch {
      toast.push("warning", "Couldn't copy automatically — select and copy it manually.");
    }
  }

  // Save the recovery key straight to a text file via the native save dialog,
  // so the user doesn't have to copy/paste it by hand. A successful save also
  // satisfies the "I saved it" gate so they can continue.
  async function saveRecoveryKeyToFile() {
    if (savingKey) return;
    savingKey = true;
    try {
      const destination = await save({
        title: "Save recovery key",
        defaultPath: "keyhaven-recovery-key.txt",
        filters: [{ name: "Text file", extensions: ["txt"] }],
      });
      if (!destination) return; // user cancelled the dialog
      await api.saveRecoveryKey(destination, recoveryKey);
      savedConfirmed = true;
      toast.push("success", "Recovery key saved. Keep the file somewhere safe.");
    } catch {
      toast.push("danger", "Couldn't save the recovery key to a file.");
    } finally {
      savingKey = false;
    }
  }

  function errorMessage(e: unknown): string {
    if (e && typeof e === "object" && "code" in e) {
      const code = (e as { code: string }).code;
      if (code === "io") {
        const msg = (e as { message?: string }).message;
        return msg ? `Couldn't write the vault file: ${msg}` : "Couldn't write the vault file.";
      }
      if (code === "invalidInput") {
        const msg = (e as { message?: string }).message;
        return msg ?? "That input isn't valid.";
      }
    }
    return "Something went wrong while creating the vault.";
  }

  // ---- Import an existing vault (returning user on a new device) -----------
  async function importExistingVault(): Promise<void> {
    if (importingExisting) return;
    importingExisting = true;
    try {
      const selected = await open({
        title: "Import your Keyhaven vault",
        multiple: false,
        directory: false,
        filters: [{ name: "Keyhaven Vault", extensions: ["khv"] }],
      });
      if (!selected || Array.isArray(selected)) return; // cancelled

      const destination = await resolveDefaultVaultPath();
      await api.importExternalVault(selected, destination);

      // The imported file is now this device's vault. Point at it and route to
      // Unlock so the user opens it with its ORIGINAL master password.
      vaultPath.set(destination);
      session.set({ status: "locked" });
      toast.push("success", "Vault imported. Enter your master password to unlock it.");
    } catch (e) {
      toast.push("danger", importErrorMessage(e));
    } finally {
      importingExisting = false;
    }
  }

  function importErrorMessage(e: unknown): string {
    if (e && typeof e === "object" && "code" in e) {
      const code = (e as { code: string }).code;
      if (code === "vaultCorrupted") {
        return "That file isn't a valid Keyhaven vault.";
      }
      if (code === "incompatibleVersion") {
        return "That vault was made with a newer version of Keyhaven. Update to open it.";
      }
      if (code === "invalidInput") {
        const msg = (e as { message?: string }).message;
        return msg ?? "That file can't be imported here.";
      }
      if (code === "io") {
        const msg = (e as { message?: string }).message;
        return msg ? `Couldn't import that file: ${msg}` : "Couldn't import that file.";
      }
    }
    return "Couldn't import that vault file.";
  }
</script>

<main class="view">
  <section class="card" aria-labelledby="setup-title">
    {#if step === "create"}
      <header class="head">
        <h1 id="setup-title">Create your vault</h1>
        <p class="lede">
          Choose a master password. It encrypts everything in Keyhaven and is
          never stored anywhere — keep it safe.
        </p>
      </header>

      <form class="form" onsubmit={handleCreate} novalidate>
        <div class="field">
          <label for="master-password">Master password</label>
          <div class="input-wrap">
            {#if showPassword}
              <input
                id="master-password"
                type="text"
                autocomplete="new-password"
                bind:value={password}
                aria-describedby="strength-meter"
              />
            {:else}
              <input
                id="master-password"
                type="password"
                autocomplete="new-password"
                bind:value={password}
                aria-describedby="strength-meter"
              />
            {/if}
            <button
              type="button"
              class="ghost reveal"
              aria-pressed={showPassword}
              onclick={() => (showPassword = !showPassword)}
            >
              {showPassword ? "Hide" : "Show"}
            </button>
          </div>

          <!-- Strength meter (Req 1.4) -->
          <div
            id="strength-meter"
            class="strength"
            aria-live="polite"
          >
            <div class="meter" role="presentation">
              {#each [0, 1, 2, 3] as i (i)}
                <span
                  class="seg"
                  class:filled={password.length > 0 && strength.score > i}
                  data-score={strength.score}
                ></span>
              {/each}
            </div>
            {#if strength.label}
              <span class="strength-label" data-score={strength.score}>
                {strength.label}
              </span>
            {/if}
          </div>
          {#if strength.weak}
            <p class="hint warn" role="status">
              This password is weak. You can continue, but a longer password
              with a mix of letters, numbers, and symbols is much safer.
            </p>
          {/if}
        </div>

        <div class="field">
          <label for="confirm-password">Confirm master password</label>
          <input
            id="confirm-password"
            type={showPassword ? "text" : "password"}
            autocomplete="new-password"
            bind:value={confirm}
            aria-invalid={mismatch}
            aria-describedby={mismatch ? "confirm-error" : undefined}
          />
          {#if mismatch}
            <p id="confirm-error" class="hint danger" role="alert">
              The passwords don't match.
            </p>
          {/if}
        </div>

        <!-- Recovery key option (Req 2.1) -->
        <div class="recovery-opt">
          <label class="checkbox">
            <input type="checkbox" bind:checked={generateRecovery} />
            <span>Generate a recovery key (recommended)</span>
          </label>
          {#if generateRecovery}
            <p class="hint" role="note">
              A recovery key lets you unlock this vault if you ever forget your
              master password. We'll show it once on the next screen.
            </p>
          {:else}
            <!-- Decline-path warning (Req 2.6) -->
            <p class="hint warn" role="note">
              Without a recovery key, a forgotten master password
              <strong>cannot be recovered</strong> and your data will be lost
              permanently.
            </p>
          {/if}
        </div>

        {#if error}
          <p class="form-error" role="alert">{error}</p>
        {/if}

        <button type="submit" class="primary" disabled={!canSubmit}>
          {submitting ? "Creating vault…" : "Create vault"}
        </button>
      </form>

      <!-- Returning user: import an existing vault instead of creating one. -->
      <div class="import-alt">
        <span class="import-text">Already use Keyhaven on another device?</span>
        <button
          type="button"
          class="ghost import-btn"
          onclick={importExistingVault}
          disabled={importingExisting}
        >
          {importingExisting ? "Importing…" : "Import an existing vault"}
        </button>
        <p class="hint">
          You'll unlock it with the master password (or recovery key) you already
          use — no new password needed.
        </p>
      </div>
    {:else}
      <!-- Step 2: one-time recovery-key reveal (Req 2.2, 2.4, 2.5) -->
      <header class="head">
        <h1 id="setup-title">Save your recovery key</h1>
        <p class="lede">
          This is the only time you'll see this key. Store it somewhere safe and
          separate from your computer.
        </p>
      </header>

      <div class="warning-box" role="alert">
        <strong>Important:</strong> Anyone with this recovery key has full access
        to your vault. Keep it private and back it up securely — it will not be
        shown again.
      </div>

      <div class="recovery-key">
        <code aria-label="Your recovery key">{recoveryKey}</code>
        <div class="key-actions">
          <button
            type="button"
            class="ghost"
            onclick={saveRecoveryKeyToFile}
            disabled={savingKey}
          >
            {savingKey ? "Saving…" : "Save to file"}
          </button>
          <button type="button" class="ghost" onclick={copyRecoveryKey}>
            Copy
          </button>
        </div>
      </div>

      <label class="checkbox confirm-saved">
        <input type="checkbox" bind:checked={savedConfirmed} />
        <span>I have saved my recovery key in a safe place</span>
      </label>

      <button
        type="button"
        class="primary"
        disabled={!savedConfirmed}
        onclick={finishWithRecovery}
      >
        Continue to my vault
      </button>
    {/if}
  </section>
</main>

<style>
  .view {
    min-height: 100%;
    display: grid;
    place-items: center;
    padding: var(--kh-space-6);
  }

  .card {
    width: 100%;
    max-width: 460px;
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius-lg);
    box-shadow: var(--kh-shadow);
    padding: var(--kh-space-6);
  }

  .head {
    margin-bottom: var(--kh-space-5);
  }

  .lede {
    color: var(--kh-text-muted);
    font-size: var(--kh-font-size-sm);
    margin: 0;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-5);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
  }

  label {
    font-size: var(--kh-font-size-sm);
    font-weight: var(--kh-font-weight-medium);
    color: var(--kh-text);
  }

  input[type="password"],
  input[type="text"] {
    width: 100%;
    padding: var(--kh-space-3) var(--kh-space-4);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border-strong);
    border-radius: var(--kh-radius);
    color: var(--kh-text);
    transition: border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease);
  }

  input[type="password"]:focus-visible,
  input[type="text"]:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  .input-wrap {
    position: relative;
    display: flex;
    align-items: center;
  }

  .input-wrap input {
    padding-right: 64px;
  }

  .reveal {
    position: absolute;
    right: var(--kh-space-2);
    font-size: var(--kh-font-size-sm);
  }

  /* ---- Strength meter ---- */
  .strength {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
  }

  .meter {
    display: flex;
    gap: var(--kh-space-1);
    flex: 1;
  }

  .seg {
    height: 6px;
    flex: 1;
    border-radius: var(--kh-radius-pill);
    background: var(--kh-border);
    transition: background-color var(--kh-motion) var(--kh-ease);
  }

  .seg.filled[data-score="1"] {
    background: var(--kh-danger);
  }
  .seg.filled[data-score="2"] {
    background: var(--kh-warning);
  }
  .seg.filled[data-score="3"] {
    background: var(--kh-accent);
  }
  .seg.filled[data-score="4"] {
    background: var(--kh-success);
  }

  .strength-label {
    font-size: var(--kh-font-size-xs);
    color: var(--kh-text-muted);
    min-width: 64px;
    text-align: right;
  }
  .strength-label[data-score="1"] {
    color: var(--kh-danger);
  }
  .strength-label[data-score="2"] {
    color: var(--kh-warning);
  }
  .strength-label[data-score="4"] {
    color: var(--kh-success);
  }

  /* ---- Hints / messages ---- */
  .hint {
    font-size: var(--kh-font-size-xs);
    color: var(--kh-text-muted);
    margin: 0;
    line-height: var(--kh-line-height);
  }

  .hint.warn {
    color: var(--kh-warning);
  }

  .hint.danger {
    color: var(--kh-danger);
  }

  .form-error {
    margin: 0;
    padding: var(--kh-space-3) var(--kh-space-4);
    background: var(--kh-danger-bg);
    color: var(--kh-danger);
    border-radius: var(--kh-radius);
    font-size: var(--kh-font-size-sm);
  }

  /* ---- Recovery option ---- */
  .recovery-opt {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
  }

  .checkbox {
    display: flex;
    align-items: center;
    gap: var(--kh-space-2);
    font-weight: var(--kh-font-weight-regular);
    cursor: pointer;
  }

  .checkbox input {
    width: 16px;
    height: 16px;
    accent-color: var(--kh-accent);
    cursor: pointer;
  }

  /* ---- Reveal step ---- */
  .warning-box {
    background: var(--kh-warning-bg);
    color: var(--kh-text);
    border: 1px solid var(--kh-warning);
    border-radius: var(--kh-radius);
    padding: var(--kh-space-4);
    font-size: var(--kh-font-size-sm);
    margin-bottom: var(--kh-space-5);
  }

  .recovery-key {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
    background: var(--kh-surface-sunken);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    padding: var(--kh-space-4);
    margin-bottom: var(--kh-space-5);
  }

  .recovery-key code {
    flex: 1;
    font-family: var(--kh-font-mono);
    font-size: var(--kh-font-size-md);
    word-break: break-all;
    user-select: all;
    color: var(--kh-text);
  }

  .key-actions {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
  }

  .key-actions .ghost {
    white-space: nowrap;
    text-align: center;
  }

  .confirm-saved {
    margin-bottom: var(--kh-space-5);
    font-size: var(--kh-font-size-sm);
  }

  /* ---- Import existing vault ---- */
  .import-alt {
    margin-top: var(--kh-space-5);
    padding-top: var(--kh-space-5);
    border-top: 1px solid var(--kh-border);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--kh-space-3);
  }

  .import-text {
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
  }

  .import-btn {
    padding: var(--kh-space-2) var(--kh-space-4);
  }

  /* ---- Buttons ---- */
  .primary {
    width: 100%;
    padding: var(--kh-space-3) var(--kh-space-4);
    background: var(--kh-accent);
    color: var(--kh-on-accent);
    border: none;
    border-radius: var(--kh-radius);
    font-weight: var(--kh-font-weight-semibold);
    transition: background-color var(--kh-motion-fast) var(--kh-ease);
  }

  .primary:hover:not(:disabled) {
    background: var(--kh-accent-hover);
  }

  .ghost {
    background: transparent;
    border: 1px solid var(--kh-border-strong);
    border-radius: var(--kh-radius-sm);
    padding: var(--kh-space-1) var(--kh-space-3);
    color: var(--kh-text-muted);
    transition: border-color var(--kh-motion-fast) var(--kh-ease),
      color var(--kh-motion-fast) var(--kh-ease);
  }

  .ghost:hover {
    border-color: var(--kh-accent);
    color: var(--kh-accent);
  }
</style>
