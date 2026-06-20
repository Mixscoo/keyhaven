<script lang="ts">
  /*
   * Recovery screen — Task 10.2.
   *
   * The path for a user who has forgotten their master password but still holds
   * a valid recovery key (Req 2.7). Two steps:
   *   1. "key": enter the recovery key. On success the backend unlocks the vault
   *      (returning a summary with `unlockedViaRecovery = true`) and we hold the
   *      key transiently so it can serve as the "current" secret for step 2.
   *   2. "newPassword": choose a new master password (entered twice, with the
   *      same live strength meter as Setup). `changeMasterPassword` rewraps the
   *      vault's key under the new password; the recovery key keeps working.
   *      On success we enter the unlocked shell.
   *
   * An incorrect recovery key shows a clear error and does not unlock (Req 3.3
   * analogue, Req 2.8); corrupted/IO failures are surfaced gracefully.
   *
   * This screen is rendered in place by Unlock.svelte (the app has no dedicated
   * route to it). `onBack` returns the user to password entry.
   */
  import { get } from "svelte/store";
  import * as api from "../lib/api";
  import type { VaultSummary } from "../lib/types";
  import { vaultPath, enterUnlocked } from "../lib/stores/session";
  import { toast } from "../lib/stores/toast";

  interface Props {
    /** Return to the password-unlock screen. */
    onBack: () => void;
  }

  let { onBack }: Props = $props();

  type Step = "key" | "newPassword";
  let step = $state<Step>("key");

  // Step 1 — recovery key entry.
  let recoveryKey = $state("");
  let showKey = $state(false);

  // Step 2 — new master password.
  let password = $state("");
  let confirm = $state("");
  let showPassword = $state(false);

  let submitting = $state(false);
  let error = $state("");

  // Carries the summary from the recovery unlock so the final step can enter the
  // unlocked shell once the new password is set.
  let pendingSummary = $state<VaultSummary | null>(null);

  // --- Password strength (client-side heuristic, mirrors Setup) -------------
  interface Strength {
    score: number; // 0..4
    label: string;
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

    if (pw.length < 8) score = Math.min(score, 1);
    score = Math.min(score, 4);

    const labels = ["Very weak", "Weak", "Fair", "Good", "Strong"];
    return { score, label: labels[score], weak: score < 2 };
  }

  const strength = $derived(estimateStrength(password));
  const mismatch = $derived(confirm.length > 0 && password !== confirm);

  const canUnlock = $derived(recoveryKey.trim().length > 0 && !submitting);
  const canSetPassword = $derived(
    password.length > 0 && confirm.length > 0 && !mismatch && !submitting,
  );

  // --- Actions --------------------------------------------------------------
  async function handleRecoveryUnlock(event: SubmitEvent) {
    event.preventDefault();
    error = "";

    if (recoveryKey.trim().length === 0) {
      error = "Enter your recovery key.";
      return;
    }

    submitting = true;
    try {
      const path = get(vaultPath);
      const summary = await api.unlockWithRecoveryKey(recoveryKey.trim(), path);
      // Unlocked via recovery — now require a new master password (Req 2.7).
      pendingSummary = summary;
      step = "newPassword";
    } catch (e) {
      error = errorMessage(e, "key");
    } finally {
      submitting = false;
    }
  }

  async function handleSetPassword(event: SubmitEvent) {
    event.preventDefault();
    error = "";

    if (password.length === 0) {
      error = "Enter a new master password.";
      return;
    }
    if (password !== confirm) {
      error = "The passwords don't match. Please re-enter them.";
      return;
    }
    if (!pendingSummary) {
      // Defensive: shouldn't happen, but avoid entering in an unknown state.
      error = "Your recovery session expired. Please enter your recovery key again.";
      step = "key";
      return;
    }

    submitting = true;
    try {
      // The recovery key is the "current" secret accepted by the backend after a
      // recovery unlock; this rewraps the vault key under the new password.
      await api.changeMasterPassword(recoveryKey.trim(), password);

      const summary = pendingSummary;
      // Drop all secrets from memory before leaving the screen.
      recoveryKey = "";
      password = "";
      confirm = "";
      pendingSummary = null;

      toast.push("success", "Your master password has been updated.");
      await enterUnlocked(summary);
    } catch (e) {
      error = errorMessage(e, "newPassword");
    } finally {
      submitting = false;
    }
  }

  function errorMessage(e: unknown, context: Step): string {
    if (e && typeof e === "object" && "code" in e) {
      const code = (e as { code: string }).code;
      if (code === "wrongCredentials") {
        return context === "key"
          ? "Incorrect recovery key. Please check it and try again."
          : "We couldn't verify your recovery key. Please start over.";
      }
      if (code === "vaultCorrupted") {
        return "This vault file appears to be damaged or isn't a Keyhaven vault. It couldn't be opened.";
      }
      if (code === "incompatibleVersion") {
        return "This vault was created with a newer version of Keyhaven and can't be opened here.";
      }
      if (code === "invalidInput") {
        const msg = (e as { message?: string }).message;
        return msg ?? "That input isn't valid.";
      }
      if (code === "io") {
        const msg = (e as { message?: string }).message;
        return msg ? `Couldn't access the vault file: ${msg}` : "Couldn't access the vault file.";
      }
    }
    return context === "key"
      ? "Something went wrong while unlocking with your recovery key."
      : "Something went wrong while setting your new master password.";
  }
</script>

<main class="view">
  <section class="card" aria-labelledby="recovery-title">
    {#if step === "key"}
      <header class="head">
        <h1 id="recovery-title">Use your recovery key</h1>
        <p class="lede">
          Enter the recovery key you saved when you created this vault. After
          unlocking, you'll set a new master password.
        </p>
      </header>

      <form class="form" onsubmit={handleRecoveryUnlock} novalidate>
        <div class="field">
          <label for="recovery-key">Recovery key</label>
          <div class="input-wrap">
            {#if showKey}
              <input
                id="recovery-key"
                type="text"
                autocomplete="off"
                spellcheck="false"
                bind:value={recoveryKey}
                aria-invalid={error.length > 0}
                aria-describedby={error ? "recovery-error" : undefined}
              />
            {:else}
              <input
                id="recovery-key"
                type="password"
                autocomplete="off"
                spellcheck="false"
                bind:value={recoveryKey}
                aria-invalid={error.length > 0}
                aria-describedby={error ? "recovery-error" : undefined}
              />
            {/if}
            <button
              type="button"
              class="ghost reveal"
              aria-pressed={showKey}
              onclick={() => (showKey = !showKey)}
            >
              {showKey ? "Hide" : "Show"}
            </button>
          </div>
        </div>

        {#if error}
          <p id="recovery-error" class="form-error" role="alert">{error}</p>
        {/if}

        <button type="submit" class="primary" disabled={!canUnlock}>
          {submitting ? "Unlocking…" : "Unlock with recovery key"}
        </button>
      </form>

      <div class="alt">
        <button type="button" class="link" onclick={onBack}>
          Back to master password
        </button>
      </div>
    {:else}
      <header class="head">
        <h1 id="recovery-title">Set a new master password</h1>
        <p class="lede">
          Your vault is unlocked. Choose a new master password — it will replace
          the one you forgot. Your recovery key will keep working.
        </p>
      </header>

      <form class="form" onsubmit={handleSetPassword} novalidate>
        <div class="field">
          <label for="new-password">New master password</label>
          <div class="input-wrap">
            {#if showPassword}
              <input
                id="new-password"
                type="text"
                autocomplete="new-password"
                bind:value={password}
                aria-describedby="strength-meter"
              />
            {:else}
              <input
                id="new-password"
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

          <!-- Strength meter (mirrors Setup, Req 1.4 pattern) -->
          <div id="strength-meter" class="strength" aria-live="polite">
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
          <label for="confirm-password">Confirm new master password</label>
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

        {#if error}
          <p class="form-error" role="alert">{error}</p>
        {/if}

        <button type="submit" class="primary" disabled={!canSetPassword}>
          {submitting ? "Saving…" : "Set new password and open vault"}
        </button>
      </form>
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

  .alt {
    margin-top: var(--kh-space-5);
    text-align: center;
  }

  .link {
    background: transparent;
    border: none;
    color: var(--kh-accent);
    font-size: var(--kh-font-size-sm);
    padding: var(--kh-space-1) var(--kh-space-2);
    border-radius: var(--kh-radius-sm);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .link:hover {
    color: var(--kh-accent-hover);
  }

  .link:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }
</style>
