<script lang="ts">
  /*
   * EntryEditor — Task 12.1 (service picker + field prefill).
   *
   * Two entry paths:
   *   - Create mode (route has no `entryId`): show the ServicePicker first
   *     (Req 5.1). When a service is chosen, build the initial working field
   *     set. Catalog services prefill their recommended fields (Req 5.2, 7.1);
   *     custom services start with an empty field set the user will define in
   *     task 12.2 (Req 6.5).
   *   - Edit mode (route carries an `entryId`): load the existing entry via
   *     `getEntry` and skip the picker entirely.
   *
   * This task lays in the service selection and the prefilled working-field
   * data structure (with generated temp ids) and shows it. The interactive
   * field editing, masking/reveal, copy, generator, save, and delete arrive in
   * task 12.2 — hence the read-only preview below.
   */
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import * as api from "../lib/api";
  import { route, openList } from "../lib/stores/navigation";
  import { toast } from "../lib/stores/toast";
  import { settings } from "../lib/stores/settings";
  import { loadEntries } from "../lib/stores/entries";
  import { debouncedSearchQuery } from "../lib/stores/searchQuery";
  import ServicePicker from "../lib/components/ServicePicker.svelte";
  import FieldRow from "../lib/components/FieldRow.svelte";
  import Modal from "../lib/components/Modal.svelte";
  import ServiceIcon from "../lib/components/ServiceIcon.svelte";
  import { catalogById, ensureCatalogLoaded } from "../lib/stores/catalog";
  import { validateFieldValue } from "../lib/validation";
  import type {
    Entry,
    EntryInput,
    FieldInput,
    RecommendedField,
    ServiceRef,
    ServiceSelection,
    WorkingField,
  } from "../lib/types";

  // The id of the entry being edited (create mode when absent). Captured once
  // on mount so a later route change doesn't reshape the open editor.
  const entryId = $derived.by(() =>
    $route.name === "editor" ? $route.entryId : undefined,
  );
  const isEdit = $derived(entryId !== undefined);

  // ---- Editor working state ------------------------------------------------
  let loading = $state(false);
  // Null until a service is chosen (create mode) or the entry loads (edit mode).
  let serviceRef = $state<ServiceRef | null>(null);
  let serviceName = $state("");
  let isCustomService = $state(false);
  let title = $state("");
  let fields = $state<WorkingField[]>([]);
  // Serialized snapshot of the last-saved/loaded state, used to detect whether
  // the form has unsaved changes (dirty). Set when a service is chosen (create)
  // or an entry loads (edit), and refreshed after a successful save.
  let baseline = $state<string>("");

  /** Generate a stable temp id for a freshly added/prefilled field row. */
  function tempId(): string {
    if (
      typeof crypto !== "undefined" &&
      typeof crypto.randomUUID === "function"
    ) {
      return `tmp-${crypto.randomUUID()}`;
    }
    return `tmp-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  }

  /** Map a service's recommended fields into empty, editable working fields. */
  function fieldsFromRecommended(recommended: RecommendedField[]): WorkingField[] {
    return recommended.map((rf) => ({
      id: tempId(),
      label: rf.label,
      type: rf.type,
      value: "",
      secret: rf.secret,
    }));
  }

  /** Handle a selection emitted by the ServicePicker (create mode). */
  function handleServiceSelected(selection: ServiceSelection): void {
    serviceRef = selection.serviceRef;
    serviceName = selection.name;
    isCustomService = selection.custom;
    // Prefill recommended fields (catalog); custom services start empty.
    fields = fieldsFromRecommended(selection.recommendedFields);
    // The freshly prefilled (empty) form is the clean baseline for dirty checks.
    baseline = snapshotOf(title, fields);
  }

  /** Populate the working state from an existing entry (edit mode). */
  function loadFromEntry(entry: Entry): void {
    serviceRef = entry.service_ref;
    isCustomService = entry.service_ref.kind === "custom";
    serviceName =
      entry.title?.trim() ||
      (entry.service_ref.kind === "catalog"
        ? prettify(entry.service_ref.id)
        : "Custom service");
    title = entry.title ?? "";
    fields = entry.fields.map((f) => ({
      id: f.id,
      label: f.label,
      type: f.type,
      value: f.value,
      secret: f.secret,
    }));
    // The just-loaded entry is the clean baseline for dirty checks.
    baseline = snapshotOf(title, fields);
  }

  function prettify(id: string): string {
    return id
      .replace(/[-_]+/g, " ")
      .replace(/\b\w/g, (c) => c.toUpperCase())
      .trim();
  }

  /** Return to the service picker (create mode) to choose a different service. */
  function changeService(): void {
    serviceRef = null;
    serviceName = "";
    isCustomService = false;
    title = "";
    fields = [];
  }

  // ---- Field editing (Req 7.2, 7.3, 7.5) -----------------------------------
  let saving = $state(false);
  let deleting = $state(false);
  let confirmDelete = $state(false);
  // Shown when the user tries to leave the editor with unsaved changes.
  let confirmBack = $state(false);

  /** Best-effort activity ping so backend auto-lock doesn't fire mid-edit. */
  function pingActivity(): void {
    void api.reportActivity().catch(() => {});
  }

  /** Append a new, empty text field for the user to fill in (Req 7.2). */
  function addField(): void {
    pingActivity();
    fields = [
      ...fields,
      { id: tempId(), label: "", type: "text", value: "", secret: false },
    ];
  }

  /** Remove the field at `index` (Req 7.3). */
  function removeField(index: number): void {
    pingActivity();
    fields = fields.filter((_, i) => i !== index);
  }

  /** Apply a partial edit to the field at `index`. */
  function updateField(index: number, patch: Partial<WorkingField>): void {
    fields = fields.map((f, i) => (i === index ? { ...f, ...patch } : f));
  }

  // ---- Validation (Req 5.6 hardened) ---------------------------------------
  // Per-field validation: a field with a value must have a label and a value
  // that's valid for its type. Empty fields don't error individually — the
  // form-level rule below requires at least one filled field instead.
  function errorForField(f: WorkingField): string | null {
    const value = f.value.trim();
    if (value.length === 0) return null;
    if (f.label.trim().length === 0) return "Give this field a label.";
    return validateFieldValue(f.type, value);
  }

  const fieldErrors = $derived(fields.map(errorForField));
  const hasFieldErrors = $derived(fieldErrors.some((e) => e !== null));
  // An entry must carry at least one filled-in value to be worth saving.
  const hasAnyValue = $derived(fields.some((f) => f.value.trim().length > 0));

  /** A stable, content-only serialization used to compare against the baseline. */
  function snapshotOf(t: string, fs: WorkingField[]): string {
    return JSON.stringify({
      title: t,
      fields: fs.map((f) => ({
        label: f.label,
        type: f.type,
        value: f.value,
        secret: f.secret,
      })),
    });
  }

  // True when the form differs from the last-saved/loaded state. Always false
  // before a service is chosen (nothing to lose on the picker screen).
  const dirty = $derived(
    serviceRef !== null && snapshotOf(title, fields) !== baseline,
  );

  const canSave = $derived(
    serviceRef !== null && !saving && !hasFieldErrors && hasAnyValue && dirty,
  );
  // Why saving is blocked, shown beside the action when it can't proceed.
  const blockReason = $derived(
    !serviceRef
      ? ""
      : !hasAnyValue
        ? "Fill in at least one field value before saving."
        : hasFieldErrors
          ? "Fix the highlighted fields before saving."
          : !dirty
            ? "No changes to save yet."
            : "",
  );

  // ---- Save (Req 5.3, 5.6, 7.6, 8.5) ---------------------------------------
  /**
   * Map the working fields to backend `FieldInput`. Fields left blank are
   * dropped entirely so an entry never persists empty fields; temp ids are
   * omitted so the backend treats those as new fields. (Saving is already
   * blocked unless at least one field has a value, so this never yields an
   * empty entry.)
   */
  function buildInput(): EntryInput {
    const fieldInputs: FieldInput[] = fields
      .filter((f) => f.value.trim().length > 0)
      .map((f) => {
        const input: FieldInput = {
          label: f.label,
          type: f.type,
          value: f.value,
          secret: f.secret,
        };
        // Existing fields keep their real id; temp ids are new fields (omit id).
        if (!f.id.startsWith("tmp-")) input.id = f.id;
        return input;
      });
    const trimmedTitle = title.trim();
    return {
      service_ref: serviceRef as ServiceRef,
      title: trimmedTitle.length > 0 ? trimmedTitle : undefined,
      fields: fieldInputs,
    };
  }

  async function save(): Promise<void> {
    if (!serviceRef || saving) return;
    if (!canSave) {
      if (blockReason) toast.push("warning", blockReason);
      return;
    }
    pingActivity();
    saving = true;
    try {
      const input = buildInput();
      const result = isEdit
        ? await api.updateEntry(entryId as string, input)
        : await api.createEntry(input);

      // The backend already saved; an empty entry only warrants a warning (Req 5.6).
      if (result.emptyWarning) {
        toast.push("warning", "Saved an empty entry — it has no field values.");
      } else {
        toast.push("success", isEdit ? "Entry updated." : "Entry saved.");
      }

      // Refresh the list so it reflects the change, then return to it.
      await loadEntries(get(debouncedSearchQuery)).catch(() => {});
      openList();
    } catch {
      toast.push("danger", "Could not save this entry.");
    } finally {
      saving = false;
    }
  }

  // ---- Delete (Req 8.6) ----------------------------------------------------
  function requestDelete(): void {
    pingActivity();
    confirmDelete = true;
  }

  function cancelDelete(): void {
    confirmDelete = false;
  }

  // ---- Leave guard (Back with unsaved changes) -----------------------------
  /** Back action: confirm before leaving if there are unsaved changes. */
  function requestBack(): void {
    if (dirty) {
      confirmBack = true;
    } else {
      openList();
    }
  }

  function confirmBackDiscard(): void {
    confirmBack = false;
    openList();
  }

  function cancelBack(): void {
    confirmBack = false;
  }

  async function performDelete(): Promise<void> {
    if (!isEdit || deleting) return;
    pingActivity();
    deleting = true;
    try {
      await api.deleteEntry(entryId as string);
      toast.push("success", "Entry deleted.");
      await loadEntries(get(debouncedSearchQuery)).catch(() => {});
      openList();
    } catch {
      toast.push("danger", "Could not delete this entry.");
    } finally {
      deleting = false;
      confirmDelete = false;
    }
  }

  onMount(() => {
    if (!isEdit) return;
    const id = entryId;
    if (!id) return;
    loading = true;
    api
      .getEntry(id)
      .then((entry) => loadFromEntry(entry))
      .catch(() => {
        toast.push("danger", "Could not load this entry.");
        openList();
      })
      .finally(() => {
        loading = false;
      });
  });

  // In create mode the picker is shown until a service is chosen.
  const showPicker = $derived(!isEdit && serviceRef === null);

  // The catalog logo for the chosen service (both create and edit modes), looked
  // up by id from the lazily-loaded catalog cache.
  const catalogSvc = $derived(
    serviceRef && serviceRef.kind === "catalog"
      ? $catalogById[serviceRef.id]
      : undefined,
  );

  onMount(() => {
    void ensureCatalogLoaded();
  });
</script>

<section class="editor">
  <header class="topbar">
    <button class="btn ghost" type="button" onclick={requestBack}>‹ Back</button>
    <h1>{isEdit ? "Edit entry" : "New entry"}</h1>
  </header>

  {#if loading}
    <p class="status">Loading…</p>
  {:else if showPicker}
    <p class="lede">Choose the service this credential belongs to.</p>
    <ServicePicker onSelect={handleServiceSelected} />
  {:else if serviceRef}
    <div class="chosen">
      <div class="service-head">
        <ServiceIcon
          name={serviceName}
          id={serviceRef.kind === "catalog" ? serviceRef.id : ""}
          svg={catalogSvc?.svg ?? ""}
          color={catalogSvc?.color ?? ""}
          iconData={catalogSvc?.icon_data ?? ""}
          size={44}
        />
        <div class="service-meta">
          <span class="service-name">{serviceName}</span>
          {#if isCustomService}
            <span class="badge">Custom</span>
          {/if}
        </div>
        {#if !isEdit}
          <button class="btn ghost change" type="button" onclick={changeService}>
            Change service
          </button>
        {/if}
      </div>

      <!-- Optional entry title (Req 5.4 association uses service; title is a
           human label the user can set). -->
      <div class="title-field">
        <label for="entry-title">Title (optional)</label>
        <input
          id="entry-title"
          type="text"
          placeholder={serviceName}
          autocomplete="off"
          bind:value={title}
          oninput={pingActivity}
        />
      </div>

      <!-- Interactive field editing: add/remove/relabel, mask/reveal, copy,
           inline generator (Req 5.3, 7.2–7.5, 8.2, 8.3, 10.1). -->
      <div class="fields">
        <h3 class="fields-title">Fields</h3>
        {#if fields.length === 0}
          <p class="status">No fields yet. Add one below.</p>
        {:else}
          <div class="field-list">
            {#each fields as field, index (field.id)}
              <FieldRow
                {field}
                entryId={isEdit ? entryId : undefined}
                genDefaults={$settings.password_gen_defaults}
                error={fieldErrors[index]}
                onChange={(patch) => updateField(index, patch)}
                onRemove={() => removeField(index)}
                onActivity={pingActivity}
              />
            {/each}
          </div>
        {/if}

        <button class="btn ghost add-field" type="button" onclick={addField}>
          <span class="plus" aria-hidden="true">+</span>
          Add field
        </button>
      </div>

      <!-- Save / delete actions -->
      <footer class="editor-actions">
        <button
          class="btn primary"
          type="button"
          onclick={save}
          disabled={saving || !canSave}
        >
          {saving ? "Saving…" : isEdit ? "Save changes" : "Save entry"}
        </button>
        {#if isEdit}
          <button
            class="btn danger"
            type="button"
            onclick={requestDelete}
            disabled={deleting}
          >
            Delete
          </button>
        {/if}
        {#if blockReason}
          <p class="block-reason" role="status">{blockReason}</p>
        {/if}
      </footer>
    </div>
  {/if}
</section>

<!-- Unsaved-changes guard on leaving the editor -->
<Modal open={confirmBack}>
  <div class="confirm">
    <h3 class="confirm-title">Discard changes?</h3>
    <p class="confirm-body">
      You have unsaved changes. If you go back now, they'll be lost.
    </p>
    <div class="confirm-actions">
      <button class="btn ghost" type="button" onclick={cancelBack}>
        Keep editing
      </button>
      <button class="btn danger" type="button" onclick={confirmBackDiscard}>
        Discard changes
      </button>
    </div>
  </div>
</Modal>

<!-- Delete confirmation (Req 8.6) -->
<Modal open={confirmDelete}>
  <div class="confirm">
    <h3 class="confirm-title">Delete this entry?</h3>
    <p class="confirm-body">
      This permanently removes “{serviceName}” and all its fields from your
      vault. This cannot be undone.
    </p>
    <div class="confirm-actions">
      <button
        class="btn ghost"
        type="button"
        onclick={cancelDelete}
        disabled={deleting}
      >
        Cancel
      </button>
      <button
        class="btn danger"
        type="button"
        onclick={performDelete}
        disabled={deleting}
      >
        {deleting ? "Deleting…" : "Delete"}
      </button>
    </div>
  </div>
</Modal>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-4);
    min-height: 100%;
    padding: var(--kh-space-5) var(--kh-space-6) var(--kh-space-6);
    max-width: 640px;
    margin: 0 auto;
    width: 100%;
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
  }

  .topbar h1 {
    margin: 0;
    font-size: var(--kh-font-size-xl);
  }

  .lede {
    margin: 0;
    color: var(--kh-text-muted);
    font-size: var(--kh-font-size-sm);
  }

  .status {
    margin: 0;
    color: var(--kh-text-muted);
    font-size: var(--kh-font-size-sm);
  }

  /* ---- Chosen service ---- */
  .chosen {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-5);
  }

  .service-head {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
    padding: var(--kh-space-4);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
  }

  .service-meta {
    flex: 1 1 auto;
    display: flex;
    align-items: center;
    gap: var(--kh-space-2);
    min-width: 0;
  }

  .service-name {
    font-weight: var(--kh-font-weight-semibold);
    font-size: var(--kh-font-size-lg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .badge {
    flex: 0 0 auto;
    font-size: var(--kh-font-size-xs);
    font-weight: var(--kh-font-weight-medium);
    padding: 2px var(--kh-space-2);
    border-radius: var(--kh-radius-pill);
    background: var(--kh-accent-subtle);
    color: var(--kh-accent-hover);
  }

  .change {
    flex: 0 0 auto;
  }

  /* ---- Title field ---- */
  .title-field {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
  }

  .title-field label {
    font-size: var(--kh-font-size-sm);
    font-weight: var(--kh-font-weight-medium);
    color: var(--kh-text);
  }

  .title-field input {
    width: 100%;
    padding: var(--kh-space-3) var(--kh-space-4);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    color: var(--kh-text);
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease);
  }

  .title-field input:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  /* ---- Fields editor ---- */
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-3);
  }

  .fields-title {
    margin: 0;
    font-size: var(--kh-font-size-xs);
    font-weight: var(--kh-font-weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--kh-text-subtle);
  }

  .field-list {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
  }

  .add-field {
    align-self: flex-start;
  }

  /* ---- Save / delete actions ---- */
  .editor-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--kh-space-3);
    padding-top: var(--kh-space-2);
    border-top: 1px solid var(--kh-border);
  }

  .block-reason {
    margin: 0;
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
  }

  /* ---- Delete confirmation ---- */
  .confirm {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-3);
    max-width: 360px;
  }

  .confirm-title {
    margin: 0;
    font-size: var(--kh-font-size-lg);
  }

  .confirm-body {
    margin: 0;
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--kh-space-2);
  }

  .plus {
    font-size: 1.1em;
    line-height: 1;
  }

  /* ---- Buttons ---- */
  .btn {
    display: inline-flex;
    align-items: center;
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

  .btn.ghost {
    background: var(--kh-surface);
    border-color: var(--kh-border);
    color: var(--kh-text);
  }

  .btn.ghost:hover {
    border-color: var(--kh-border-strong);
    background: var(--kh-surface-sunken);
  }

  .btn.primary {
    background: var(--kh-accent);
    color: var(--kh-on-accent);
  }

  .btn.primary:hover:not(:disabled) {
    background: var(--kh-accent-hover);
  }

  .btn.danger {
    background: var(--kh-surface);
    border-color: var(--kh-border);
    color: var(--kh-danger);
  }

  .btn.danger:hover:not(:disabled) {
    border-color: var(--kh-danger);
    background: var(--kh-danger-bg);
  }

  .btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
