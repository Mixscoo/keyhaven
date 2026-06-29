<script lang="ts">
  /*
   * FieldRow — an editable row for a single entry field (Task 12.2).
   *
   * Responsibilities:
   *   - Edit the field's label (Req 7.5), type, and value (Req 5.3).
   *   - Toggle whether the field is a secret; secret fields are masked by
   *     default with a reveal control (Req 7.4, 8.2).
   *   - When revealed, offer a copy-to-clipboard control (Req 8.3). For
   *     already-saved secret fields we route through the backend
   *     `copy_secret_to_clipboard` so its auto-clear timer applies (Req 8.4);
   *     for unsaved values we copy the in-memory value directly.
   *   - Offer an inline password generator on secret fields (Req 10.1).
   *   - Remove the field (Req 7.3).
   *
   * The row is "controlled": it never mutates the field object itself; instead
   * it reports edits via `onChange` so the editor owns the working state.
   */
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import * as api from "../api";
  import { toast } from "../stores/toast";
  import type { FieldType, PasswordGenOptions, WorkingField } from "../types";
  import PasswordGenerator from "./PasswordGenerator.svelte";
  import Select from "./Select.svelte";

  let {
    field,
    entryId,
    genDefaults,
    error,
    readonly = false,
    onChange,
    onRemove,
    onActivity,
  }: {
    field: WorkingField;
    /** Saved entry id, if this row belongs to a persisted entry. */
    entryId?: string;
    /** Generator defaults seeded from the vault settings. */
    genDefaults: PasswordGenOptions;
    /** Validation error for this field, or null/undefined when valid. */
    error?: string | null;
    /** Read-only view: show value with reveal/copy, but no editing controls. */
    readonly?: boolean;
    /** Report a field edit (partial patch) back to the editor. */
    onChange: (patch: Partial<WorkingField>) => void;
    /** Remove this field row. */
    onRemove: () => void;
    /** Best-effort activity ping to keep auto-lock from firing mid-edit. */
    onActivity?: () => void;
  } = $props();

  // Selectable field types with friendly labels.
  const FIELD_TYPES: { value: FieldType; label: string }[] = [
    { value: "email", label: "Email" },
    { value: "username", label: "Username" },
    { value: "password", label: "Password" },
    { value: "phone", label: "Phone" },
    { value: "url", label: "URL" },
    { value: "text", label: "Text" },
    { value: "note", label: "Note" },
    { value: "totp_secret", label: "2FA secret" },
    { value: "recovery_code", label: "Recovery code" },
  ];

  /** The friendly label for a field type (e.g. "email" → "Email"). */
  function labelForType(type: FieldType): string {
    return FIELD_TYPES.find((ft) => ft.value === type)?.label ?? "";
  }

  /** All default type labels, used to detect an untouched/auto label. */
  const DEFAULT_LABELS = new Set(FIELD_TYPES.map((ft) => ft.label));

  let revealed = $state(false);
  let showGenerator = $state(false);

  // A field is persisted (so the backend can copy + auto-clear it) only when it
  // belongs to a saved entry and carries a real (non-temp) id.
  const isSaved = $derived(
    entryId !== undefined && !field.id.startsWith("tmp-"),
  );

  function patch(part: Partial<WorkingField>): void {
    onActivity?.();
    onChange(part);
  }

  function onLabelInput(event: Event): void {
    patch({ label: (event.currentTarget as HTMLInputElement).value });
  }

  function onValueInput(event: Event): void {
    patch({ value: (event.currentTarget as HTMLInputElement).value });
  }

  function onTypeChange(type: FieldType): void {
    // Default new secret-ish types to masked; keep explicit user choice otherwise.
    const secretByDefault =
      type === "password" || type === "totp_secret" || type === "recovery_code";

    // Keep the visible label in step with the type unless the user has typed
    // their own custom label. A label is considered "auto" when it's empty or
    // still equals one of the default type labels (e.g. "Email"); in that case
    // switching Email → Password also relabels the row "Password".
    const labelIsAuto =
      field.label.trim().length === 0 || DEFAULT_LABELS.has(field.label.trim());

    patch({
      type,
      secret: secretByDefault ? true : field.secret,
      ...(labelIsAuto ? { label: labelForType(type) } : {}),
    });
  }

  function toggleSecret(event: Event): void {
    const secret = (event.currentTarget as HTMLInputElement).checked;
    if (!secret) revealed = false;
    patch({ secret });
  }

  function toggleReveal(): void {
    onActivity?.();
    revealed = !revealed;
  }

  async function copyValue(): Promise<void> {
    onActivity?.();
    try {
      if (isSaved && field.secret) {
        // Persisted secret: let the backend copy + schedule auto-clear (Req 8.4).
        await api.copySecretToClipboard(entryId as string, field.id);
      } else {
        // Unsaved value (or non-secret): copy the in-memory value directly.
        await writeText(field.value);
      }
      toast.push("success", "Copied to clipboard.");
    } catch {
      toast.push("danger", "Could not copy to clipboard.");
    }
  }

  function openGenerator(): void {
    onActivity?.();
    showGenerator = true;
  }

  function useGenerated(password: string): void {
    patch({ value: password });
    revealed = true;
    showGenerator = false;
  }
</script>

<div class="field-row" class:readonly>
  <div class="line">
    {#if readonly}
      <span class="ro-label">{field.label}</span>
    {:else}
      <input
        class="label-input"
        type="text"
        placeholder="Label"
        aria-label="Field label"
        value={field.label}
        oninput={onLabelInput}
      />

      <div class="type-select">
        <Select
          value={field.type}
          options={FIELD_TYPES}
          onChange={onTypeChange}
          ariaLabel="Field type"
          minWidth="140px"
        />
      </div>

      <button
        class="icon-btn remove"
        type="button"
        onclick={onRemove}
        aria-label="Remove field"
        title="Remove field"
      >
        <!-- close / remove -->
        <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
          <line x1="5.5" y1="5.5" x2="14.5" y2="14.5" />
          <line x1="14.5" y1="5.5" x2="5.5" y2="14.5" />
        </svg>
      </button>
    {/if}
  </div>

  <div class="line value-line">
    <input
      class="value-input"
      class:invalid={!!error}
      type={field.secret && !revealed ? "password" : "text"}
      placeholder="Value"
      aria-label="Field value"
      aria-invalid={!!error}
      autocomplete="off"
      spellcheck="false"
      value={field.value}
      oninput={onValueInput}
      readonly={readonly}
    />

    {#if field.secret}
      <button
        class="icon-btn"
        type="button"
        onclick={toggleReveal}
        aria-label={revealed ? "Hide value" : "Reveal value"}
        aria-pressed={revealed}
        title={revealed ? "Hide" : "Reveal"}
      >
        {#if revealed}
          <!-- eye-off -->
          <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M8.1 4.7A7.4 7.4 0 0 1 10 4.5c5 0 8 5.5 8 5.5a14 14 0 0 1-2 2.7M5.2 5.8A13.3 13.3 0 0 0 2 10s3 5.5 8 5.5a7.3 7.3 0 0 0 2.9-.6" />
            <path d="M8.3 8.3a2.4 2.4 0 0 0 3.4 3.4" />
            <line x1="3" y1="3" x2="17" y2="17" />
          </svg>
        {:else}
          <!-- eye -->
          <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M2 10S5 4.5 10 4.5 18 10 18 10 15 15.5 10 15.5 2 10 2 10Z" />
            <circle cx="10" cy="10" r="2.4" />
          </svg>
        {/if}
      </button>
    {/if}

    {#if !field.secret || revealed || readonly}
      <button
        class="icon-btn"
        type="button"
        onclick={copyValue}
        aria-label="Copy value to clipboard"
        title="Copy"
        disabled={field.value.length === 0}
      >
        <!-- copy -->
        <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="7" y="7" width="9" height="9" rx="2" />
          <path d="M13 7V5.5A1.5 1.5 0 0 0 11.5 4H5.5A1.5 1.5 0 0 0 4 5.5v6A1.5 1.5 0 0 0 5.5 13H7" />
        </svg>
      </button>
    {/if}

    {#if field.secret && !readonly}
      <button
        class="icon-btn"
        type="button"
        onclick={openGenerator}
        aria-label="Generate password"
        title="Generate password"
      >
        <!-- sparkles (generate) -->
        <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M9 3.2l1.3 3 3 1.3-3 1.3L9 11.8 7.7 8.8 4.7 7.5l3-1.3z" />
          <path d="M14.8 12l.7 1.6 1.6.7-1.6.7-.7 1.6-.7-1.6-1.6-.7 1.6-.7z" />
        </svg>
      </button>
    {/if}
  </div>

  {#if !readonly}
    <label class="secret-toggle">
      <input type="checkbox" checked={field.secret} onchange={toggleSecret} />
      <span>Secret (mask this value)</span>
    </label>
  {/if}

  {#if error}
    <p class="field-error" role="alert">{error}</p>
  {/if}

  {#if showGenerator}
    <PasswordGenerator
      initial={genDefaults}
      onUse={useGenerated}
      onCancel={() => (showGenerator = false)}
      {onActivity}
    />
  {/if}
</div>

<style>
  .field-row {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
    padding: var(--kh-space-3);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
  }

  .line {
    display: flex;
    align-items: center;
    gap: var(--kh-space-2);
  }

  .label-input {
    flex: 1 1 auto;
    min-width: 0;
  }

  .type-select {
    flex: 0 0 auto;
  }

  .value-input {
    flex: 1 1 auto;
    min-width: 0;
    font-family: var(--kh-font-mono);
  }

  .value-input.invalid {
    border-color: var(--kh-danger);
  }

  .value-input.invalid:focus-visible {
    border-color: var(--kh-danger);
    box-shadow: 0 0 0 3px var(--kh-danger-bg);
  }

  .field-error {
    margin: 0;
    font-size: var(--kh-font-size-xs);
    color: var(--kh-danger);
  }

  input[type="text"],
  input[type="password"] {
    padding: var(--kh-space-2) var(--kh-space-3);
    background-color: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius-sm);
    color: var(--kh-text);
    font-size: var(--kh-font-size-sm);
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease);
  }

  input[type="text"]:focus-visible,
  input[type="password"]:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  /* Read-only (View) appearance: inputs look like static text, not editable. */
  .field-row.readonly input[readonly] {
    background-color: var(--kh-surface-sunken);
    cursor: default;
  }

  /* The field label in View mode is a small caption, not a text field. */
  .ro-label {
    font-size: var(--kh-font-size-xs);
    font-weight: var(--kh-font-weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--kh-text-subtle);
  }

  .icon-btn {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius-sm);
    color: var(--kh-text);
    cursor: pointer;
    transition:
      background var(--kh-motion-fast) var(--kh-ease),
      border-color var(--kh-motion-fast) var(--kh-ease);
  }

  .icon-btn:hover:not(:disabled) {
    border-color: var(--kh-border-strong);
    background: var(--kh-surface-sunken);
  }

  .icon-btn:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  .icon-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .icon-btn svg {
    width: 17px;
    height: 17px;
  }

  .icon-btn.remove:hover:not(:disabled) {
    border-color: var(--kh-danger);
    color: var(--kh-danger);
    background: var(--kh-danger-bg);
  }

  .secret-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--kh-space-2);
    font-size: var(--kh-font-size-xs);
    color: var(--kh-text-muted);
    cursor: pointer;
  }

  .secret-toggle input {
    accent-color: var(--kh-accent);
  }
</style>
