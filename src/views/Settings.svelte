<script lang="ts">
  /*
   * Settings screen — Task 13.
   *
   * Surfaces the user-tunable preferences that are persisted (encrypted) inside
   * the vault, plus the export/import + backup guidance:
   *   - Auto-lock timeout, including an explicit "disable" option (Req 4.3).
   *   - Lock-on-blur toggle (Req 4.5 surface; backend owns the behavior).
   *   - Clipboard-clear delay (Req 8.4).
   *   - Password-generator defaults: length + charset toggles (seed the inline
   *     generator in the editor).
   *   - Export / back up to a chosen file via the native save dialog (Req 11.1)
   *     and import from a chosen file via the native open dialog (Req 11.3),
   *     with prominent "keep backups in multiple safe places" guidance (Req 11.5).
   *
   * Everything that mutates a setting routes through `commit`, which persists via
   * the backend (`update_settings`) and updates the store optimistically,
   * reverting on failure. All cryptography and file I/O happen in the Rust
   * backend; this screen only collects input and calls the typed command
   * wrappers. Layout follows the calm, uncluttered design tokens (Req 14.5).
   */
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { getVersion } from "@tauri-apps/api/app";
  import * as api from "../lib/api";
  import { reportActivity } from "../lib/api";
  import { settings, loadSettings, saveSettings } from "../lib/stores/settings";
  import { toast } from "../lib/stores/toast";
  import { openList } from "../lib/stores/navigation";
  import Select from "../lib/components/Select.svelte";
  import type { Settings } from "../lib/types";

  // Sensible bounds for the generated-password length control. The backend is
  // authoritative; these just keep the slider usable and match the inline
  // generator's range.
  const MIN_LENGTH = 8;
  const MAX_LENGTH = 64;

  // Discrete auto-lock choices. `0` disables auto-lock entirely (Req 4.3).
  const AUTO_LOCK_OPTIONS: { value: number; label: string }[] = [
    { value: 60, label: "1 minute" },
    { value: 300, label: "5 minutes" },
    { value: 900, label: "15 minutes" },
    { value: 1800, label: "30 minutes" },
    { value: 3600, label: "1 hour" },
    { value: 0, label: "Never (disable auto-lock)" },
  ];

  // Short, configurable clipboard-clear delays (Req 8.4).
  const CLIPBOARD_OPTIONS: { value: number; label: string }[] = [
    { value: 10, label: "10 seconds" },
    { value: 20, label: "20 seconds" },
    { value: 30, label: "30 seconds" },
    { value: 45, label: "45 seconds" },
    { value: 60, label: "1 minute" },
  ];

  // Local working mirror of the store. Seeded synchronously from the store
  // (already loaded by the unlocked shell), then refreshed on mount.
  let current = $state<Settings>(clone(get(settings)));

  let saving = $state(false);
  // True briefly after a successful save so we can show a calm "Saved" note.
  let justSaved = $state(false);
  let exporting = $state(false);
  let importing = $state(false);
  let savedTimer: ReturnType<typeof setTimeout> | undefined;
  // The running app version (from Tauri), shown so users know what they have.
  let appVersion = $state("");

  function clone(s: Settings): Settings {
    return {
      ...s,
      password_gen_defaults: { ...s.password_gen_defaults },
    };
  }

  function clampLength(n: number): number {
    if (!Number.isFinite(n)) return 20;
    return Math.min(MAX_LENGTH, Math.max(MIN_LENGTH, Math.round(n)));
  }

  onMount(() => {
    // Refresh from the vault so the screen reflects the persisted truth even if
    // the store hasn't been hydrated yet this session.
    void loadSettings()
      .then(() => {
        current = clone(get(settings));
      })
      .catch(() => {
        toast.push("warning", "Couldn't load your saved settings.");
      });
    // Surface the running app version (best-effort; non-fatal if unavailable).
    void getVersion()
      .then((v) => {
        appVersion = v;
      })
      .catch(() => {});
  });

  /**
   * Persist `next` to the vault and update the store. Optimistic: we apply the
   * change locally first, then revert if the write fails so the UI never shows
   * an unsaved value as if it were saved.
   */
  async function commit(next: Settings): Promise<void> {
    const previous = current;
    current = next;
    saving = true;
    reportActivity().catch(() => {});
    try {
      await saveSettings(next);
      justSaved = true;
      clearTimeout(savedTimer);
      savedTimer = setTimeout(() => {
        justSaved = false;
      }, 2500);
    } catch {
      current = previous;
      toast.push("danger", "Couldn't save that setting. Please try again.");
    } finally {
      saving = false;
    }
  }

  // ---- Setting handlers ----------------------------------------------------

  function onAutoLockChange(value: number): void {
    void commit({ ...clone(current), auto_lock_seconds: value });
  }

  function onLockOnBlurChange(event: Event): void {
    const checked = (event.currentTarget as HTMLInputElement).checked;
    void commit({ ...clone(current), lock_on_blur: checked });
  }

  function onClipboardChange(value: number): void {
    void commit({ ...clone(current), clipboard_clear_seconds: value });
  }

  // Live length value for the slider/stepper. Updated immediately on drag (no
  // save), and persisted via `commitLength` on release / typed entry / +/-.
  let lengthValue = $state(get(settings).password_gen_defaults.length);
  $effect(() => {
    lengthValue = current.password_gen_defaults.length;
  });

  /** Persist a new length (clamped) to the vault. */
  function commitLength(n: number): void {
    const value = clampLength(n);
    lengthValue = value;
    if (value === current.password_gen_defaults.length) return;
    const next = clone(current);
    next.password_gen_defaults.length = value;
    void commit(next);
  }

  /** Live update while dragging the slider (no save until release). */
  function onLengthInput(event: Event): void {
    lengthValue = clampLength(Number((event.currentTarget as HTMLInputElement).value));
  }

  type Charset = "upper" | "lower" | "digits" | "symbols";

  function selectedCharsetCount(d: Settings["password_gen_defaults"]): number {
    return [d.upper, d.lower, d.digits, d.symbols].filter(Boolean).length;
  }

  function onCharsetChange(set: Charset, event: Event): void {
    const checked = (event.currentTarget as HTMLInputElement).checked;
    // Keep at least one character set selected so generated passwords are valid.
    if (!checked && selectedCharsetCount(current.password_gen_defaults) <= 1) {
      (event.currentTarget as HTMLInputElement).checked = true;
      toast.push("warning", "Keep at least one character set selected.");
      return;
    }
    const next = clone(current);
    next.password_gen_defaults[set] = checked;
    void commit(next);
  }

  // ---- Export / import -----------------------------------------------------

  async function handleExport(): Promise<void> {
    if (exporting) return;
    reportActivity().catch(() => {});
    exporting = true;
    try {
      const destination = await save({
        title: "Save vault backup",
        defaultPath: "keyhaven-backup.khv",
        filters: [{ name: "Keyhaven Vault", extensions: ["khv"] }],
      });
      if (!destination) return; // user cancelled the dialog
      await api.exportVault(destination);
      toast.push("success", "Backup saved. Keep copies in multiple safe places.");
    } catch (e) {
      toast.push("danger", ioMessage(e, "Couldn't save the backup."));
    } finally {
      exporting = false;
    }
  }

  async function handleImport(): Promise<void> {
    if (importing) return;
    reportActivity().catch(() => {});
    importing = true;
    try {
      const selected = await open({
        title: "Import a vault file",
        multiple: false,
        directory: false,
        filters: [{ name: "Keyhaven Vault", extensions: ["khv"] }],
      });
      if (!selected || Array.isArray(selected)) return; // cancelled
      await api.importVault(selected);
      toast.push("success", "Vault file imported successfully.");
    } catch (e) {
      toast.push("danger", importMessage(e));
    } finally {
      importing = false;
    }
  }

  function ioMessage(e: unknown, fallback: string): string {
    if (e && typeof e === "object" && "code" in e) {
      const code = (e as { code: string }).code;
      if (code === "io") {
        const msg = (e as { message?: string }).message;
        return msg ? `${fallback} (${msg})` : fallback;
      }
    }
    return fallback;
  }

  function importMessage(e: unknown): string {
    if (e && typeof e === "object" && "code" in e) {
      const code = (e as { code: string }).code;
      if (code === "vaultCorrupted") {
        return "That file isn't a valid Keyhaven vault. Your current vault is unchanged.";
      }
      if (code === "incompatibleVersion") {
        return "That vault was made with a newer version of Keyhaven. Update to open it.";
      }
      if (code === "io") {
        const msg = (e as { message?: string }).message;
        return msg ? `Couldn't read that file (${msg}).` : "Couldn't read that file.";
      }
    }
    return "Couldn't import that file. Your current vault is unchanged.";
  }
</script>

<section class="view">
  <header class="head">
    <button class="ghost back" type="button" onclick={openList}>
      ← Back
    </button>
    <div class="title-wrap">
      <h1>Settings</h1>
      <p class="saved" aria-live="polite">
        {#if saving}
          Saving…
        {:else if justSaved}
          Saved
        {/if}
      </p>
    </div>
  </header>

  <!-- Security / auto-lock --------------------------------------------------->
  <section class="panel" aria-labelledby="sec-security">
    <h2 id="sec-security">Security</h2>

    <div class="row">
      <div class="row-text">
        <label for="auto-lock">Auto-lock after inactivity</label>
        <p class="hint">
          Lock the vault automatically when you step away. Choose “Never” to
          disable auto-locking.
        </p>
      </div>
      <Select
        id="auto-lock"
        value={current.auto_lock_seconds}
        options={AUTO_LOCK_OPTIONS}
        onChange={onAutoLockChange}
        ariaLabel="Auto-lock after inactivity"
        minWidth="200px"
      />
    </div>

    <div class="row">
      <div class="row-text">
        <span class="row-label" id="lock-blur-label">Lock when the window loses focus</span>
        <p class="hint">
          Also lock the vault when Keyhaven is minimized or sent to the
          background.
        </p>
      </div>
      <label class="switch" aria-labelledby="lock-blur-label">
        <input
          type="checkbox"
          checked={current.lock_on_blur}
          onchange={onLockOnBlurChange}
        />
        <span class="track" aria-hidden="true"></span>
      </label>
    </div>
  </section>

  <!-- Clipboard -------------------------------------------------------------->
  <section class="panel" aria-labelledby="sec-clipboard">
    <h2 id="sec-clipboard">Clipboard</h2>

    <div class="row">
      <div class="row-text">
        <label for="clip-clear">Clear copied secrets after</label>
        <p class="hint">
          Copied passwords and secrets are wiped from the clipboard after this
          delay.
        </p>
      </div>
      <Select
        id="clip-clear"
        value={current.clipboard_clear_seconds}
        options={CLIPBOARD_OPTIONS}
        onChange={onClipboardChange}
        ariaLabel="Clear copied secrets after"
        minWidth="200px"
      />
    </div>
  </section>

  <!-- Password generator defaults -------------------------------------------->
  <section class="panel" aria-labelledby="sec-generator">
    <h2 id="sec-generator">Password generator defaults</h2>
    <p class="hint panel-hint">
      These seed the generator when you create or edit an entry.
    </p>

    <div class="row length-row">
      <div class="row-text">
        <span class="row-label">Length: {lengthValue}</span>
        <p class="hint">
          How many characters generated passwords have ({MIN_LENGTH}–{MAX_LENGTH}).
        </p>
      </div>
      <div class="length-control">
        <button
          class="step"
          type="button"
          aria-label="Decrease length"
          onclick={() => commitLength(lengthValue - 1)}
          disabled={lengthValue <= MIN_LENGTH}
        >
          −
        </button>
        <input
          id="gen-length"
          class="range"
          type="range"
          min={MIN_LENGTH}
          max={MAX_LENGTH}
          value={lengthValue}
          oninput={onLengthInput}
          onchange={(e) => commitLength(Number((e.currentTarget as HTMLInputElement).value))}
          aria-label="Default generated password length"
        />
        <button
          class="step"
          type="button"
          aria-label="Increase length"
          onclick={() => commitLength(lengthValue + 1)}
          disabled={lengthValue >= MAX_LENGTH}
        >
          +
        </button>
        <input
          class="num"
          type="number"
          min={MIN_LENGTH}
          max={MAX_LENGTH}
          value={lengthValue}
          onchange={(e) => commitLength(Number((e.currentTarget as HTMLInputElement).value))}
          aria-label="Length value"
        />
      </div>
    </div>

    <div class="row">
      <div class="row-text">
        <span class="row-label">Character sets</span>
        <p class="hint">At least one set stays selected.</p>
      </div>
      <div class="charsets" role="group" aria-label="Character sets">
        <label class="chip" class:active={current.password_gen_defaults.upper}>
          <input
            type="checkbox"
            checked={current.password_gen_defaults.upper}
            onchange={(e) => onCharsetChange("upper", e)}
          />
          <span>A–Z</span>
        </label>
        <label class="chip" class:active={current.password_gen_defaults.lower}>
          <input
            type="checkbox"
            checked={current.password_gen_defaults.lower}
            onchange={(e) => onCharsetChange("lower", e)}
          />
          <span>a–z</span>
        </label>
        <label class="chip" class:active={current.password_gen_defaults.digits}>
          <input
            type="checkbox"
            checked={current.password_gen_defaults.digits}
            onchange={(e) => onCharsetChange("digits", e)}
          />
          <span>0–9</span>
        </label>
        <label class="chip" class:active={current.password_gen_defaults.symbols}>
          <input
            type="checkbox"
            checked={current.password_gen_defaults.symbols}
            onchange={(e) => onCharsetChange("symbols", e)}
          />
          <span>!@#</span>
        </label>
      </div>
    </div>
  </section>

  <!-- Backup / export / import ----------------------------------------------->
  <section class="panel" aria-labelledby="sec-backup">
    <h2 id="sec-backup">Backup &amp; restore</h2>

    <div class="guidance" role="note">
      <strong>Keep backups in multiple safe places.</strong>
      Your vault lives only on this device. Export it regularly and store copies
      in more than one safe location (for example an external drive and a
      separate trusted location). Backups stay encrypted — they still need your
      master password or recovery key to open.
    </div>

    <div class="row">
      <div class="row-text">
        <span class="row-label">Export / back up vault</span>
        <p class="hint">Save an encrypted copy of your vault to a location you choose.</p>
      </div>
      <button
        class="btn primary"
        type="button"
        onclick={handleExport}
        disabled={exporting}
      >
        {exporting ? "Exporting…" : "Export…"}
      </button>
    </div>

    <div class="row">
      <div class="row-text">
        <span class="row-label">Import vault file</span>
        <p class="hint">Load a vault file you previously exported.</p>
      </div>
      <button
        class="btn ghost"
        type="button"
        onclick={handleImport}
        disabled={importing}
      >
        {importing ? "Importing…" : "Import…"}
      </button>
    </div>
  </section>

  {#if appVersion}
    <footer class="app-version">Keyhaven v{appVersion}</footer>
  {/if}
</section>

<style>
  .view {
    max-width: 720px;
    margin: 0 auto;
    padding: var(--kh-space-6);
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-5);
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--kh-space-4);
  }

  .title-wrap {
    display: flex;
    align-items: baseline;
    gap: var(--kh-space-3);
  }

  .title-wrap h1 {
    margin: 0;
  }

  .saved {
    margin: 0;
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
    min-height: 1.2em;
  }

  .panel {
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius-lg);
    box-shadow: var(--kh-shadow-sm);
    padding: var(--kh-space-5);
  }

  .panel h2 {
    margin: 0 0 var(--kh-space-3);
    font-size: var(--kh-font-size-lg);
    font-weight: var(--kh-font-weight-semibold);
  }

  .panel-hint {
    margin-top: calc(-1 * var(--kh-space-2));
    margin-bottom: var(--kh-space-3);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--kh-space-5);
    padding: var(--kh-space-4) 0;
    border-top: 1px solid var(--kh-border);
  }

  .panel h2 + .row,
  .panel-hint + .row {
    border-top: none;
    padding-top: 0;
  }

  .row-text {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-1);
  }

  .row-text label,
  .row-label {
    font-size: var(--kh-font-size-md);
    font-weight: var(--kh-font-weight-medium);
    color: var(--kh-text);
  }

  .hint {
    margin: 0;
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
    line-height: var(--kh-line-height);
  }

  /* ---- Length control (slider + stepper) ---- */
  .length-row {
    flex-direction: column;
    align-items: stretch;
    gap: var(--kh-space-3);
  }

  .length-control {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
  }

  .range {
    flex: 1 1 auto;
    min-width: 0;
    height: 8px;
    margin: 0;
    border-radius: var(--kh-radius-pill);
    background: var(--kh-surface-sunken);
    border: 1px solid var(--kh-border);
    appearance: none;
    -webkit-appearance: none;
    cursor: pointer;
  }

  .range::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--kh-accent);
    border: 3px solid var(--kh-surface);
    box-shadow: var(--kh-shadow-sm);
    cursor: pointer;
    transition: background var(--kh-motion-fast) var(--kh-ease);
  }

  .range::-webkit-slider-thumb:hover {
    background: var(--kh-accent-hover);
  }

  .range::-moz-range-thumb {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--kh-accent);
    border: 3px solid var(--kh-surface);
    box-shadow: var(--kh-shadow-sm);
    cursor: pointer;
  }

  .range:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  .step {
    flex: 0 0 auto;
    width: 36px;
    height: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: var(--kh-font-size-lg);
    line-height: 1;
    background: var(--kh-surface);
    border: 1px solid var(--kh-border-strong);
    border-radius: var(--kh-radius-sm);
    color: var(--kh-text);
    cursor: pointer;
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      background var(--kh-motion-fast) var(--kh-ease);
  }

  .step:hover:not(:disabled) {
    border-color: var(--kh-accent);
    color: var(--kh-accent);
  }

  .step:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .num {
    flex: 0 0 auto;
    width: 64px;
    padding: var(--kh-space-2) var(--kh-space-3);
    text-align: center;
    background-color: var(--kh-surface);
    border: 1px solid var(--kh-border-strong);
    border-radius: var(--kh-radius-sm);
    color: var(--kh-text);
    font: inherit;
    font-weight: var(--kh-font-weight-semibold);
  }

  .num:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  /* ---- Toggle switch ---- */
  .switch {
    flex: 0 0 auto;
    position: relative;
    display: inline-flex;
    align-items: center;
    cursor: pointer;
  }

  .switch input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }

  .switch .track {
    width: 42px;
    height: 24px;
    border-radius: var(--kh-radius-pill);
    background: var(--kh-border-strong);
    transition: background var(--kh-motion-fast) var(--kh-ease);
    position: relative;
  }

  .switch .track::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--kh-surface);
    box-shadow: var(--kh-shadow-sm);
    transition: transform var(--kh-motion-fast) var(--kh-ease);
  }

  .switch input:checked + .track {
    background: var(--kh-accent);
  }

  .switch input:checked + .track::after {
    transform: translateX(18px);
  }

  .switch input:focus-visible + .track {
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  /* ---- Charset toggle chips ---- */
  .charsets {
    flex: 0 0 auto;
    display: flex;
    flex-wrap: wrap;
    gap: var(--kh-space-2);
    justify-content: flex-end;
  }

  .chip {
    position: relative;
    display: inline-flex;
    align-items: center;
    padding: var(--kh-space-2) var(--kh-space-4);
    border: 1px solid var(--kh-border-strong);
    border-radius: var(--kh-radius-pill);
    background: var(--kh-surface);
    color: var(--kh-text-muted);
    font-family: var(--kh-font-mono);
    font-size: var(--kh-font-size-sm);
    font-weight: var(--kh-font-weight-medium);
    cursor: pointer;
    user-select: none;
    transition:
      background var(--kh-motion-fast) var(--kh-ease),
      border-color var(--kh-motion-fast) var(--kh-ease),
      color var(--kh-motion-fast) var(--kh-ease);
  }

  /* The native checkbox drives state but is visually hidden. */
  .chip input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }

  .chip:hover {
    border-color: var(--kh-accent);
    color: var(--kh-text);
  }

  .chip.active {
    background: var(--kh-accent-subtle);
    border-color: var(--kh-accent);
    color: var(--kh-accent-hover);
  }

  .chip:focus-within {
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  /* ---- Backup guidance ---- */
  .guidance {
    background: var(--kh-accent-subtle);
    border: 1px solid var(--kh-accent);
    border-radius: var(--kh-radius);
    padding: var(--kh-space-4);
    margin-bottom: var(--kh-space-3);
    font-size: var(--kh-font-size-sm);
    line-height: var(--kh-line-height);
    color: var(--kh-text);
  }

  .guidance strong {
    display: block;
    margin-bottom: var(--kh-space-1);
  }

  /* ---- Buttons ---- */
  .btn {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: var(--kh-space-3) var(--kh-space-4);
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

  .btn.ghost {
    background: var(--kh-surface);
    border-color: var(--kh-border-strong);
    color: var(--kh-text);
  }

  .btn.ghost:hover:not(:disabled) {
    border-color: var(--kh-accent);
    color: var(--kh-accent);
  }

  .btn:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .ghost.back {
    background: transparent;
    border: 1px solid var(--kh-border-strong);
    border-radius: var(--kh-radius-sm);
    padding: var(--kh-space-2) var(--kh-space-3);
    color: var(--kh-text-muted);
    cursor: pointer;
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      color var(--kh-motion-fast) var(--kh-ease);
  }

  .ghost.back:hover {
    border-color: var(--kh-accent);
    color: var(--kh-accent);
  }

  .app-version {
    text-align: center;
    font-size: var(--kh-font-size-xs);
    color: var(--kh-text-subtle);
    padding-top: var(--kh-space-2);
  }

  @media (max-width: 560px) {
    .row {
      flex-direction: column;
      align-items: stretch;
    }

    .charsets,
    .range {
      justify-content: flex-start;
      max-width: none;
      flex-basis: auto;
    }
  }
</style>
