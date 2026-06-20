<script lang="ts">
  /*
   * ServicePicker — Task 12.1.
   *
   * The first step of creating an entry: pick the service the credential
   * belongs to. It offers three things in one calm surface:
   *   1. A searchable list of bundled catalog services (Req 5.1, 12.2). Search
   *      is debounced (~120ms) so we don't query the backend on every keystroke.
   *   2. The user's previously created custom services, marked with a "Custom"
   *      badge (Req 6.3).
   *   3. A "Create custom service" flow (name + optional icon) for anything not
   *      in the catalog (Req 6.1, 6.2).
   *
   * On selection it emits a `ServiceSelection` via `onSelect` — bundling the
   * `serviceRef`, a display name, and (for catalog services) the recommended
   * fields the EntryEditor uses to prefill the form (Req 5.2, 7.1). Custom
   * services carry no recommended fields; the editor starts those empty.
   *
   * Icons: bundled catalog icons aren't reliably renderable through the Tauri
   * asset pipeline yet, so — matching EntryCard — we show a calm monogram avatar
   * derived from the service name/id. It's cheap, always available, and gives
   * every service a stable, recognizable tint.
   */
  import { onMount } from "svelte";
  import * as api from "../api";
  import { toast } from "../stores/toast";
  import ServiceIcon from "./ServiceIcon.svelte";
  import type {
    CatalogService,
    CustomService,
    IconRef,
    ServiceSelection,
  } from "../types";

  let {
    onSelect,
  }: {
    onSelect: (selection: ServiceSelection) => void;
  } = $props();

  // ---- Browse / search state ----------------------------------------------
  let query = $state("");
  // Debounced mirror of `query` that actually drives the backend search.
  let debouncedQuery = $state("");
  let results = $state<CatalogService[]>([]);
  let customServices = $state<CustomService[]>([]);
  let searching = $state(false);

  // ---- Create-custom flow state --------------------------------------------
  let mode = $state<"browse" | "create-custom">("browse");
  let customName = $state("");
  // Optional icon for a new custom service, captured as a data URL via a plain
  // file input (no network, no Tauri dependency). Empty = monogram fallback.
  let customIcon = $state<IconRef | null>(null);
  let customIconPreview = $state("");
  let creating = $state(false);

  // Debounce the search input (~120ms), mirroring the searchQuery store pattern.
  const SEARCH_DEBOUNCE_MS = 120;
  $effect(() => {
    const value = query;
    const timer = setTimeout(() => {
      debouncedQuery = value;
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  });

  // Run the catalog search whenever the debounced query settles. An empty query
  // surfaces the default/popular catalog list so there's always something to
  // pick from (Req 5.1).
  $effect(() => {
    const q = debouncedQuery;
    searching = true;
    let cancelled = false;
    api
      .searchCatalog(q)
      .then((list) => {
        if (!cancelled) results = list;
      })
      .catch(() => {
        if (!cancelled) {
          results = [];
          toast.push("danger", "Could not load the service catalog.");
        }
      })
      .finally(() => {
        if (!cancelled) searching = false;
      });
    return () => {
      cancelled = true;
    };
  });

  onMount(() => {
    void loadCustomServices();
  });

  async function loadCustomServices(): Promise<void> {
    try {
      customServices = await api.listCustomServices();
    } catch {
      toast.push("warning", "Could not load your custom services.");
    }
  }

  // ---- Avatar helpers (mirrors EntryCard for visual consistency) -----------
  function prettify(id: string): string {
    return id
      .replace(/[-_]+/g, " ")
      .replace(/\b\w/g, (c) => c.toUpperCase())
      .trim();
  }

  function hueOf(s: string): number {
    let h = 0;
    for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 360;
    return h;
  }

  function monogramOf(name: string): string {
    return (name.trim()[0] ?? "?").toUpperCase();
  }

  // ---- Selection handlers --------------------------------------------------
  function chooseCatalog(service: CatalogService): void {
    onSelect({
      serviceRef: { kind: "catalog", id: service.id },
      name: service.name,
      custom: false,
      recommendedFields: service.recommended_fields,
    });
  }

  function chooseCustom(service: CustomService): void {
    onSelect({
      serviceRef: { kind: "custom", id: service.id },
      name: service.name,
      custom: true,
      // Custom services have no recommended fields; the editor starts empty.
      recommendedFields: [],
    });
  }

  // ---- Create-custom flow --------------------------------------------------
  function openCreateCustom(): void {
    // Seed the name with whatever the user was searching for — likely the
    // service that wasn't in the catalog.
    customName = query.trim();
    customIcon = null;
    customIconPreview = "";
    mode = "create-custom";
  }

  function cancelCreateCustom(): void {
    mode = "browse";
    customName = "";
    customIcon = null;
    customIconPreview = "";
  }

  function onIconChange(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) {
      customIcon = null;
      customIconPreview = "";
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = typeof reader.result === "string" ? reader.result : "";
      if (dataUrl) {
        customIcon = { kind: "data", ref: dataUrl };
        customIconPreview = dataUrl;
      }
    };
    reader.onerror = () => {
      toast.push("warning", "Couldn't read that image. A monogram will be used.");
      customIcon = null;
      customIconPreview = "";
    };
    reader.readAsDataURL(file);
  }

  async function submitCustom(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const name = customName.trim();
    if (name.length === 0) {
      toast.push("warning", "Give your custom service a name.");
      return;
    }
    creating = true;
    try {
      // Fall back to a builtin (empty) icon ref when none was provided; the UI
      // renders a monogram in that case.
      const icon: IconRef = customIcon ?? { kind: "builtin", ref: "" };
      const created = await api.createCustomService(name, icon);
      customServices = [...customServices, created];
      toast.push("success", `Added “${created.name}”.`);
      chooseCustom(created);
    } catch {
      toast.push("danger", "Could not create the custom service.");
    } finally {
      creating = false;
    }
  }
</script>

<div class="service-picker">
  {#if mode === "browse"}
    <div class="search">
      <svg
        class="search-icon"
        viewBox="0 0 20 20"
        aria-hidden="true"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
      >
        <circle cx="9" cy="9" r="6" />
        <line x1="13.5" y1="13.5" x2="17" y2="17" stroke-linecap="round" />
      </svg>
      <input
        type="search"
        placeholder="Search services…"
        aria-label="Search the service catalog"
        autocomplete="off"
        spellcheck="false"
        bind:value={query}
      />
    </div>

    {#if customServices.length > 0}
      <div class="group">
        <h3 class="group-title">Your custom services</h3>
        <ul class="list">
          {#each customServices as service (service.id)}
            <li>
              <button
                class="service-row"
                type="button"
                onclick={() => chooseCustom(service)}
              >
                {#if service.icon.kind === "data" && service.icon.ref}
                  <ServiceIcon name={service.name} imgSrc={service.icon.ref} />
                {:else}
                  <ServiceIcon name={service.name} id={service.id} />
                {/if}
                <span class="name">{service.name}</span>
                <span class="badge">Custom</span>
              </button>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    <div class="group">
      <h3 class="group-title">Catalog</h3>
      {#if searching && results.length === 0}
        <p class="status">Searching…</p>
      {:else if results.length === 0}
        <p class="status">
          No catalog services match “{query.trim()}”.
        </p>
      {:else}
        <ul class="list">
          {#each results as service (service.id)}
            <li>
              <button
                class="service-row"
                type="button"
                onclick={() => chooseCatalog(service)}
              >
                <ServiceIcon
                  name={service.name}
                  id={service.id}
                  svg={service.svg ?? ""}
                  color={service.color ?? ""}
                  iconData={service.icon_data ?? ""}
                />
                <span class="name">{service.name}</span>
                {#if service.recommended_fields.length > 0}
                  <span class="field-hint">
                    {service.recommended_fields.length} fields
                  </span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="create-row">
      <span class="create-hint">Can't find it?</span>
      <button class="btn ghost" type="button" onclick={openCreateCustom}>
        <span class="plus" aria-hidden="true">+</span>
        Create custom service
      </button>
    </div>
  {:else}
    <!-- Create custom service (Req 6.1, 6.2) -->
    <form class="custom-form" onsubmit={submitCustom}>
      <header class="custom-head">
        <button
          class="btn ghost back"
          type="button"
          onclick={cancelCreateCustom}
        >
          ‹ Back
        </button>
        <h3>Create custom service</h3>
      </header>

      <div class="field">
        <label for="custom-name">Service name</label>
        <input
          id="custom-name"
          type="text"
          placeholder="e.g. My Home Server"
          autocomplete="off"
          bind:value={customName}
        />
      </div>

      <div class="field">
        <span class="label">Icon (optional)</span>
        <div class="icon-picker">
          {#if customIconPreview}
            <ServiceIcon name={customName || "?"} imgSrc={customIconPreview} />
          {:else}
            <ServiceIcon name={customName || "?"} id={customName || "custom"} />
          {/if}
          <label class="btn ghost file-btn">
            Choose image…
            <input
              type="file"
              accept="image/*"
              onchange={onIconChange}
              hidden
            />
          </label>
          {#if customIconPreview}
            <button
              class="btn ghost"
              type="button"
              onclick={() => {
                customIcon = null;
                customIconPreview = "";
              }}
            >
              Remove
            </button>
          {/if}
        </div>
        <p class="hint">
          No icon needed — a calm monogram is used when none is chosen.
        </p>
      </div>

      <button
        class="btn primary submit"
        type="submit"
        disabled={creating || customName.trim().length === 0}
      >
        {creating ? "Creating…" : "Create and use"}
      </button>
    </form>
  {/if}
</div>

<style>
  .service-picker {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-4);
  }

  /* ---- Search ---- */
  .search {
    position: relative;
  }

  .search-icon {
    position: absolute;
    left: var(--kh-space-3);
    top: 50%;
    transform: translateY(-50%);
    width: 18px;
    height: 18px;
    color: var(--kh-text-subtle);
    pointer-events: none;
  }

  .search input {
    width: 100%;
    padding: var(--kh-space-3) var(--kh-space-4) var(--kh-space-3)
      var(--kh-space-7);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    color: var(--kh-text);
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease);
  }

  .search input:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  /* ---- Groups & lists ---- */
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
  }

  .group-title {
    margin: 0;
    font-size: var(--kh-font-size-xs);
    font-weight: var(--kh-font-weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--kh-text-subtle);
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-1);
    max-height: 320px;
    overflow-y: auto;
  }

  .service-row {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
    width: 100%;
    padding: var(--kh-space-2) var(--kh-space-3);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    text-align: left;
    color: var(--kh-text);
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease);
  }

  .service-row:hover {
    border-color: var(--kh-border-strong);
    box-shadow: var(--kh-shadow-sm);
  }

  .service-row:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  .name {
    flex: 1 1 auto;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: var(--kh-font-weight-medium);
  }

  .field-hint {
    flex: 0 0 auto;
    font-size: var(--kh-font-size-xs);
    color: var(--kh-text-subtle);
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

  .status {
    margin: 0;
    padding: var(--kh-space-2) var(--kh-space-1);
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
  }

  /* ---- Create row ---- */
  .create-row {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
    padding-top: var(--kh-space-2);
    border-top: 1px solid var(--kh-border);
  }

  .create-hint {
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
  }

  /* ---- Create-custom form ---- */
  .custom-form {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-4);
  }

  .custom-head {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
  }

  .custom-head h3 {
    margin: 0;
    font-size: var(--kh-font-size-lg);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
  }

  .field label,
  .field .label {
    font-size: var(--kh-font-size-sm);
    font-weight: var(--kh-font-weight-medium);
    color: var(--kh-text);
  }

  .field input[type="text"] {
    width: 100%;
    padding: var(--kh-space-3) var(--kh-space-4);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border-strong);
    border-radius: var(--kh-radius);
    color: var(--kh-text);
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease);
  }

  .field input[type="text"]:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  .icon-picker {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
  }

  .hint {
    margin: 0;
    font-size: var(--kh-font-size-xs);
    color: var(--kh-text-muted);
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

  .btn.ghost:hover {
    border-color: var(--kh-border-strong);
    background: var(--kh-surface-sunken);
  }

  .file-btn {
    cursor: pointer;
  }

  .back {
    padding: var(--kh-space-1) var(--kh-space-2);
  }

  .submit {
    align-self: flex-start;
    padding: var(--kh-space-3) var(--kh-space-5);
  }

  .plus {
    font-size: 1.1em;
    line-height: 1;
  }
</style>
