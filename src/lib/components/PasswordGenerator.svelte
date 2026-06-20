<script lang="ts">
  /*
   * PasswordGenerator — inline strong-password generator (Req 10.1–10.4).
   *
   * Offered inline on password/secret fields. The actual randomness comes from
   * the trusted Rust backend (`generate_password`, which uses a CSPRNG — Req
   * 10.3); this component is purely a control surface:
   *   - length (number + range slider) and charset toggles (upper/lower/digits/
   *     symbols) — Req 10.2,
   *   - a preview of the generated password,
   *   - Regenerate (Req 10.4) and Use (accept into the field — Req 10.4).
   *
   * Defaults are seeded from the caller (the vault's `password_gen_defaults`).
   * The frontend NEVER generates the password itself.
   */
  import { untrack } from "svelte";
  import * as api from "../api";
  import { toast } from "../stores/toast";
  import type { PasswordGenOptions } from "../types";

  let {
    initial,
    onUse,
    onCancel,
    onActivity,
  }: {
    /** Seed options (from the vault's password_gen_defaults). */
    initial: PasswordGenOptions;
    /** Called with the generated password when the user accepts it. */
    onUse: (password: string) => void;
    /** Called when the user dismisses the generator without accepting. */
    onCancel?: () => void;
    /** Best-effort activity ping to keep auto-lock from firing mid-edit. */
    onActivity?: () => void;
  } = $props();

  // Sensible bounds for the length control. The backend is authoritative; these
  // just keep the slider usable.
  const MIN_LENGTH = 8;
  const MAX_LENGTH = 64;

  // Seed the editable controls once from the caller's defaults. `untrack` keeps
  // these initializers from reactively tracking the `initial` prop — we only
  // want its value at open time.
  let length = $state(untrack(() => clampLength(initial.length)));
  let upper = $state(untrack(() => initial.upper));
  let lower = $state(untrack(() => initial.lower));
  let digits = $state(untrack(() => initial.digits));
  let symbols = $state(untrack(() => initial.symbols));

  let preview = $state("");
  let generating = $state(false);

  function clampLength(n: number): number {
    if (!Number.isFinite(n)) return 20;
    return Math.min(MAX_LENGTH, Math.max(MIN_LENGTH, Math.round(n)));
  }

  // At least one charset must be selected for a meaningful password.
  const hasCharset = $derived(upper || lower || digits || symbols);

  function currentOptions(): PasswordGenOptions {
    return { length: clampLength(length), upper, lower, digits, symbols };
  }

  async function regenerate(): Promise<void> {
    if (!hasCharset) {
      preview = "";
      return;
    }
    onActivity?.();
    generating = true;
    try {
      preview = await api.generatePassword(currentOptions());
    } catch {
      preview = "";
      toast.push("danger", "Could not generate a password.");
    } finally {
      generating = false;
    }
  }

  function useIt(): void {
    if (!preview) return;
    onActivity?.();
    onUse(preview);
  }

  // Generate an initial preview as soon as the generator opens.
  $effect(() => {
    if (preview === "") void regenerate();
  });
</script>

<div class="generator" role="group" aria-label="Password generator">
  <div class="preview-row">
    <output class="preview" aria-live="polite">
      {#if generating}
        Generating…
      {:else if preview}
        {preview}
      {:else}
        Select at least one character set.
      {/if}
    </output>
    <button
      class="btn ghost"
      type="button"
      onclick={regenerate}
      disabled={generating || !hasCharset}
      aria-label="Regenerate password"
      title="Regenerate"
    >
      ↻
    </button>
  </div>

  <div class="controls">
    <label class="length">
      <span class="control-label">Length: {length}</span>
      <input
        type="range"
        min={MIN_LENGTH}
        max={MAX_LENGTH}
        bind:value={length}
        oninput={regenerate}
        aria-label="Password length"
      />
    </label>

    <div class="charsets">
      <label class="charset">
        <input type="checkbox" bind:checked={upper} onchange={regenerate} />
        <span>A-Z</span>
      </label>
      <label class="charset">
        <input type="checkbox" bind:checked={lower} onchange={regenerate} />
        <span>a-z</span>
      </label>
      <label class="charset">
        <input type="checkbox" bind:checked={digits} onchange={regenerate} />
        <span>0-9</span>
      </label>
      <label class="charset">
        <input type="checkbox" bind:checked={symbols} onchange={regenerate} />
        <span>!@#</span>
      </label>
    </div>
  </div>

  <div class="actions">
    <button
      class="btn primary"
      type="button"
      onclick={useIt}
      disabled={!preview || generating}
    >
      Use password
    </button>
    {#if onCancel}
      <button class="btn ghost" type="button" onclick={() => onCancel?.()}>
        Cancel
      </button>
    {/if}
  </div>
</div>

<style>
  .generator {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-3);
    padding: var(--kh-space-3);
    background: var(--kh-surface-sunken);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
  }

  .preview-row {
    display: flex;
    align-items: center;
    gap: var(--kh-space-2);
  }

  .preview {
    flex: 1 1 auto;
    min-width: 0;
    font-family: var(--kh-font-mono);
    font-size: var(--kh-font-size-sm);
    padding: var(--kh-space-2) var(--kh-space-3);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius-sm);
    color: var(--kh-text);
    word-break: break-all;
    overflow-wrap: anywhere;
  }

  .controls {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-3);
  }

  .length {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-1);
  }

  .control-label {
    font-size: var(--kh-font-size-xs);
    color: var(--kh-text-muted);
  }

  .length input[type="range"] {
    width: 100%;
    accent-color: var(--kh-accent);
  }

  .charsets {
    display: flex;
    flex-wrap: wrap;
    gap: var(--kh-space-3);
  }

  .charset {
    display: inline-flex;
    align-items: center;
    gap: var(--kh-space-1);
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text);
    cursor: pointer;
  }

  .charset input {
    accent-color: var(--kh-accent);
  }

  .actions {
    display: flex;
    gap: var(--kh-space-2);
  }

  /* ---- Buttons ---- */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--kh-space-1);
    padding: var(--kh-space-2) var(--kh-space-3);
    border-radius: var(--kh-radius);
    border: 1px solid transparent;
    font-weight: var(--kh-font-weight-medium);
    cursor: pointer;
    transition:
      background var(--kh-motion-fast) var(--kh-ease),
      border-color var(--kh-motion-fast) var(--kh-ease),
      color var(--kh-motion-fast) var(--kh-ease);
  }

  .btn.primary {
    background: var(--kh-accent);
    color: var(--kh-on-accent);
  }

  .btn.primary:hover:not(:disabled) {
    background: var(--kh-accent-hover);
  }

  .btn.primary:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .btn.ghost {
    background: var(--kh-surface);
    border-color: var(--kh-border);
    color: var(--kh-text);
  }

  .btn.ghost:hover:not(:disabled) {
    border-color: var(--kh-border-strong);
    background: var(--kh-surface-sunken);
  }

  .btn.ghost:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
