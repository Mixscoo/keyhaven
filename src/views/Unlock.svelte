<script lang="ts">
  /*
   * Unlock screen — Task 10.2.
   *
   * Shown when a vault file exists but is locked (App.svelte routes here on
   * session status "locked"). Requests the master password (Req 3.1), and on a
   * correct password decrypts and enters the unlocked shell (Req 3.2). An
   * incorrect password shows a clear error without unlocking (Req 3.3); other
   * backend failures (corrupted/tampered vault, I/O) are surfaced gracefully
   * without crashing (Req 3.6).
   *
   * The app has no dedicated route to the Recovery screen, so this screen owns
   * the "use recovery key instead" path (Req 2.7): a local `mode` toggle renders
   * the Recovery flow in place and offers a way back to password entry.
   *
   * All cryptography happens in the Rust backend via `unlockWithPassword`; this
   * screen only collects input, calls the command, and routes on success.
   */
  import { get } from "svelte/store";
  import * as api from "../lib/api";
  import { vaultPath, enterUnlocked } from "../lib/stores/session";
  import Recovery from "./Recovery.svelte";

  // Which flow is active: normal password unlock, or the recovery-key path.
  type Mode = "password" | "recovery";
  let mode = $state<Mode>("password");

  let password = $state("");
  let showPassword = $state(false);
  let submitting = $state(false);
  let error = $state("");

  const canSubmit = $derived(password.length > 0 && !submitting);

  async function handleUnlock(event: SubmitEvent) {
    event.preventDefault();
    error = "";

    if (password.length === 0) {
      error = "Enter your master password.";
      return;
    }

    submitting = true;
    try {
      const path = get(vaultPath);
      const summary = await api.unlockWithPassword(password, path);
      // Req 3.2 — correct password: drop the secret and enter the shell.
      password = "";
      await enterUnlocked(summary);
    } catch (e) {
      // Req 3.3 / 3.6 — show a clear error and stay locked.
      error = errorMessage(e);
    } finally {
      submitting = false;
    }
  }

  function useRecovery() {
    error = "";
    password = "";
    mode = "recovery";
  }

  function backToPassword() {
    error = "";
    mode = "password";
  }

  function errorMessage(e: unknown): string {
    if (e && typeof e === "object" && "code" in e) {
      const code = (e as { code: string }).code;
      if (code === "wrongCredentials") {
        return "Incorrect master password. Please try again.";
      }
      if (code === "vaultCorrupted") {
        return "This vault file appears to be damaged or isn't a Keyhaven vault. It couldn't be opened.";
      }
      if (code === "incompatibleVersion") {
        return "This vault was created with a newer version of Keyhaven and can't be opened here.";
      }
      if (code === "io") {
        const msg = (e as { message?: string }).message;
        return msg ? `Couldn't read the vault file: ${msg}` : "Couldn't read the vault file.";
      }
    }
    return "Something went wrong while unlocking the vault.";
  }
</script>

{#if mode === "recovery"}
  <Recovery onBack={backToPassword} />
{:else}
  <main class="view">
    <section class="card" aria-labelledby="unlock-title">
      <header class="head">
        <h1 id="unlock-title">Unlock Keyhaven</h1>
        <p class="lede">
          Enter your master password to decrypt and open your vault.
        </p>
      </header>

      <form class="form" onsubmit={handleUnlock} novalidate>
        <div class="field">
          <label for="master-password">Master password</label>
          <div class="input-wrap">
            {#if showPassword}
              <input
                id="master-password"
                type="text"
                autocomplete="current-password"
                bind:value={password}
                aria-invalid={error.length > 0}
                aria-describedby={error ? "unlock-error" : undefined}
              />
            {:else}
              <input
                id="master-password"
                type="password"
                autocomplete="current-password"
                bind:value={password}
                aria-invalid={error.length > 0}
                aria-describedby={error ? "unlock-error" : undefined}
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
        </div>

        {#if error}
          <p id="unlock-error" class="form-error" role="alert">{error}</p>
        {/if}

        <button type="submit" class="primary" disabled={!canSubmit}>
          {submitting ? "Unlocking…" : "Unlock"}
        </button>
      </form>

      <!-- Recovery-key path (Req 2.7) -->
      <div class="alt">
        <button type="button" class="link" onclick={useRecovery}>
          Use recovery key instead
        </button>
      </div>
    </section>
  </main>
{/if}

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

  .form-error {
    margin: 0;
    padding: var(--kh-space-3) var(--kh-space-4);
    background: var(--kh-danger-bg);
    color: var(--kh-danger);
    border-radius: var(--kh-radius);
    font-size: var(--kh-font-size-sm);
  }

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
