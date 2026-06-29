<script lang="ts">
  /*
   * Main vault view: a search bar over the entries, with a prominent "Add"
   * action, Settings, and a manual "Lock" control.
   *
   * Entries are grouped BY SERVICE into collapsible sections, so when you keep
   * several accounts for the same service (e.g. two Google logins) they sit
   * together under one tidy, logo-headed group instead of scattering through a
   * flat list. Each account opens to a read-only View first (no accidental
   * edits).
   *
   * Data flow: the search input is bound to `searchQuery`; a debounced mirror
   * drives backend `list_entries`. The whole filtered set is loaded (a personal
   * vault is small) so grouping is correct; service names/logos come from the
   * bundled catalog and the user's custom services.
   */
  import { onMount } from "svelte";
  import {
    entries,
    entriesTotal,
    entriesLimit,
    loadEntries,
  } from "../lib/stores/entries";
  import { searchQuery, debouncedSearchQuery } from "../lib/stores/searchQuery";
  import { lock } from "../lib/stores/session";
  import { openEditor, openSettings } from "../lib/stores/navigation";
  import { toast } from "../lib/stores/toast";
  import { reportActivity, listCustomServices } from "../lib/api";
  import { catalogById, ensureCatalogLoaded } from "../lib/stores/catalog";
  import ServiceIcon from "../lib/components/ServiceIcon.svelte";
  import type { CustomService, EntrySummary } from "../lib/types";

  // Group correctly by loading the whole filtered set (personal vaults are
  // small). The backend serves from the in-memory model, so this is cheap.
  entriesLimit.set(100000);

  let loading = $state(false);
  let locking = $state(false);
  let activeQuery = $state("");
  // User custom services, for per-service names/icons in grouping.
  let customById = $state<Record<string, CustomService>>({});
  // Which multi-account groups are expanded. Default: collapsed (cleaner).
  let expanded = $state<Record<string, boolean>>({});

  /** Best-effort activity ping so backend auto-lock doesn't fire mid-browse. */
  function pingActivity(): void {
    void reportActivity().catch(() => {});
  }

  async function refresh(query: string): Promise<void> {
    loading = true;
    try {
      await loadEntries(query);
      activeQuery = query;
    } catch {
      toast.push("danger", "Could not load your entries.");
    } finally {
      loading = false;
    }
  }

  async function handleLock(): Promise<void> {
    locking = true;
    try {
      await lock();
    } catch {
      toast.push("danger", "Could not lock the vault.");
      locking = false;
    }
  }

  function handleAdd(): void {
    pingActivity();
    openEditor();
  }

  function handleOpen(id: string): void {
    pingActivity();
    openEditor(id);
  }

  function handleSettings(): void {
    pingActivity();
    openSettings();
  }

  function toggleExpand(key: string): void {
    expanded = { ...expanded, [key]: !expanded[key] };
  }

  onMount(() => {
    void ensureCatalogLoaded();
    void listCustomServices()
      .then((list) => {
        const map: Record<string, CustomService> = {};
        for (const s of list) map[s.id] = s;
        customById = map;
      })
      .catch(() => {});
  });

  // React to debounced search changes (and the initial empty query).
  let lastLoaded = $state<string | null>(null);
  $effect(() => {
    const query = $debouncedSearchQuery;
    if (query === lastLoaded) return;
    lastLoaded = query;
    pingActivity();
    void refresh(query);
  });

  function prettify(id: string): string {
    return id
      .replace(/[-_]+/g, " ")
      .replace(/\b\w/g, (c) => c.toUpperCase())
      .trim();
  }

  /** The label shown for one account within a service group. */
  function accountLabel(e: EntrySummary): string {
    return e.title && e.title.trim().length > 0 ? e.title.trim() : "Untitled";
  }

  interface ServiceGroup {
    key: string;
    name: string;
    id: string;
    svg: string;
    color: string;
    iconData: string;
    imgSrc: string;
    accounts: EntrySummary[];
  }

  /** Group the entries by service, attaching display name + logo per group. */
  function buildGroups(
    list: EntrySummary[],
    catalog: Record<string, { name: string; svg?: string; color?: string; icon_data?: string }>,
    custom: Record<string, CustomService>,
  ): ServiceGroup[] {
    const map = new Map<string, ServiceGroup>();
    for (const e of list) {
      const kind = e.serviceRef.kind;
      const id = e.serviceRef.id;
      const key = `${kind}:${id}`;
      let g = map.get(key);
      if (!g) {
        let name: string;
        let svg = "";
        let color = "";
        let iconData = "";
        let imgSrc = "";
        if (kind === "catalog") {
          const c = catalog[id];
          name = c?.name ?? prettify(id);
          svg = c?.svg ?? "";
          color = c?.color ?? "";
          iconData = c?.icon_data ?? "";
        } else {
          const cs = custom[id];
          name = cs?.name ?? "Custom service";
          if (cs?.icon.kind === "data" && cs.icon.ref) imgSrc = cs.icon.ref;
        }
        g = { key, name, id, svg, color, iconData, imgSrc, accounts: [] };
        map.set(key, g);
      }
      g.accounts.push(e);
    }
    const groups = [...map.values()];
    groups.sort((a, b) => a.name.localeCompare(b.name));
    for (const g of groups) {
      g.accounts.sort((a, b) => accountLabel(a).localeCompare(accountLabel(b)));
    }
    return groups;
  }

  const groups = $derived(buildGroups($entries, $catalogById, customById));
</script>

<section class="vault">
  <header class="topbar">
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
        placeholder="Search services, labels, and details…"
        aria-label="Search entries"
        autocomplete="off"
        spellcheck="false"
        bind:value={$searchQuery}
      />
    </div>

    <div class="actions">
      <button class="btn primary" type="button" onclick={handleAdd}>
        <span class="plus" aria-hidden="true">+</span>
        Add
      </button>
      <button
        class="btn ghost icon-btn"
        type="button"
        onclick={handleSettings}
        aria-label="Settings"
        title="Settings"
      >
        <svg
          class="gear"
          viewBox="0 0 24 24"
          aria-hidden="true"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <circle cx="12" cy="12" r="3" />
          <path
            d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"
          />
        </svg>
      </button>
      <button
        class="btn ghost icon-btn"
        type="button"
        onclick={handleLock}
        disabled={locking}
        aria-label="Lock vault"
        title="Lock vault"
      >
        <svg
          class="gear"
          viewBox="0 0 24 24"
          aria-hidden="true"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <rect x="5" y="11" width="14" height="9" rx="2" />
          <path d="M8 11V8a4 4 0 0 1 8 0v3" />
          <circle cx="12" cy="15.5" r="1.2" fill="currentColor" stroke="none" />
        </svg>
      </button>
    </div>
  </header>

  <div class="count" aria-live="polite">
    {#if loading}
      Loading…
    {:else if $entriesTotal === 0 && activeQuery.length > 0}
      No matches for “{activeQuery}”
    {:else if $entriesTotal === 0}
      No entries yet
    {:else}
      {$entriesTotal}
      {$entriesTotal === 1 ? "entry" : "entries"} in {groups.length}
      {groups.length === 1 ? "service" : "services"}
    {/if}
  </div>

  <div class="list-region">
    {#if $entries.length === 0 && !loading}
      <div class="empty">
        {#if activeQuery.length > 0}
          <p>Nothing matches your search.</p>
          <p class="hint">Try a different service name or detail.</p>
        {:else}
          <p>Your vault is empty.</p>
          <button class="btn primary" type="button" onclick={handleAdd}>
            <span class="plus" aria-hidden="true">+</span>
            Add your first entry
          </button>
        {/if}
      </div>
    {:else}
      <div class="groups">
        {#each groups as g (g.key)}
          {#if g.accounts.length === 1}
            <!-- Single account: one clean row that opens the entry directly. -->
            <button
              class="entry-row"
              type="button"
              onclick={() => handleOpen(g.accounts[0].id)}
            >
              <ServiceIcon
                name={g.name}
                id={g.id}
                svg={g.svg}
                color={g.color}
                iconData={g.iconData}
                imgSrc={g.imgSrc}
                size={40}
              />
              <span class="entry-body">
                <span class="entry-title">
                  {accountLabel(g.accounts[0]) !== "Untitled"
                    ? accountLabel(g.accounts[0])
                    : g.name}
                </span>
                <span class="entry-sub">
                  <span>{g.name}</span>
                  {#if g.accounts[0].snippet}
                    <span class="dot" aria-hidden="true">·</span>
                    <span class="snip">{g.accounts[0].snippet}</span>
                  {/if}
                </span>
              </span>
              <svg
                class="arrow"
                viewBox="0 0 20 20"
                aria-hidden="true"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M8 5l5 5-5 5" />
              </svg>
            </button>
          {:else}
            <!-- Multiple accounts: a uniform row that expands (collapsed by default). -->
            <section class="group">
              <button
                class="entry-row group-header"
                type="button"
                aria-expanded={!!expanded[g.key]}
                onclick={() => toggleExpand(g.key)}
              >
                <ServiceIcon
                  name={g.name}
                  id={g.id}
                  svg={g.svg}
                  color={g.color}
                  iconData={g.iconData}
                  imgSrc={g.imgSrc}
                  size={40}
                />
                <span class="entry-body">
                  <span class="entry-title">{g.name}</span>
                  <span class="entry-sub"><span>{g.accounts.length} accounts</span></span>
                </span>
                <svg
                  class="chevron"
                  class:open={expanded[g.key]}
                  viewBox="0 0 20 20"
                  aria-hidden="true"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.8"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path d="M6 8l4 4 4-4" />
                </svg>
              </button>

              {#if expanded[g.key]}
                <ul class="accounts">
                  {#each g.accounts as acc (acc.id)}
                    <li>
                      <button
                        class="account-row"
                        type="button"
                        onclick={() => handleOpen(acc.id)}
                      >
                        <span class="account-body">
                          <span class="account-title">{accountLabel(acc)}</span>
                          {#if acc.snippet}
                            <span class="account-snippet">{acc.snippet}</span>
                          {/if}
                        </span>
                        <svg
                          class="arrow"
                          viewBox="0 0 20 20"
                          aria-hidden="true"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="1.8"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                        >
                          <path d="M8 5l5 5-5 5" />
                        </svg>
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
          {/if}
        {/each}
      </div>
    {/if}
  </div>
</section>

<style>
  .vault {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: var(--kh-space-5) var(--kh-space-6);
    gap: var(--kh-space-3);
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: var(--kh-space-4);
  }

  .search {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
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
    padding: var(--kh-space-3) var(--kh-space-4) var(--kh-space-3) var(--kh-space-7);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease);
  }

  .search input:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  .actions {
    flex: 0 0 auto;
    display: flex;
    gap: var(--kh-space-2);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: var(--kh-space-1);
    padding: var(--kh-space-3) var(--kh-space-4);
    border-radius: var(--kh-radius);
    border: 1px solid transparent;
    font-weight: var(--kh-font-weight-medium);
    transition:
      background var(--kh-motion-fast) var(--kh-ease),
      border-color var(--kh-motion-fast) var(--kh-ease),
      color var(--kh-motion-fast) var(--kh-ease);
  }

  .btn.primary {
    background: var(--kh-accent);
    color: var(--kh-on-accent);
  }

  .btn.primary:hover {
    background: var(--kh-accent-hover);
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

  .plus {
    font-size: 1.1em;
    line-height: 1;
  }

  .gear {
    width: 18px;
    height: 18px;
  }

  .icon-btn {
    padding: var(--kh-space-3);
    aspect-ratio: 1 / 1;
    justify-content: center;
    color: var(--kh-text-muted);
  }

  .icon-btn:hover {
    color: var(--kh-accent);
  }

  .count {
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
  }

  .list-region {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }

  .empty {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--kh-space-3);
    color: var(--kh-text-muted);
    text-align: center;
  }

  .empty .hint {
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-subtle);
  }

  /* ---- Grouped list ---- */
  .groups {
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
    padding-bottom: var(--kh-space-4);
  }

  /* Single-account services: one clean standalone row. */
  .entry-row {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
    width: 100%;
    padding: var(--kh-space-3) var(--kh-space-4);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    text-align: left;
    color: var(--kh-text);
    cursor: pointer;
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease),
      background var(--kh-motion-fast) var(--kh-ease);
  }

  .entry-row:hover {
    border-color: var(--kh-border-strong);
    box-shadow: var(--kh-shadow-sm);
  }

  .entry-row:focus-visible {
    outline: none;
    border-color: var(--kh-accent);
    box-shadow: 0 0 0 3px var(--kh-accent-subtle);
  }

  .entry-body {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .entry-title {
    font-weight: var(--kh-font-weight-semibold);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .entry-sub {
    display: flex;
    align-items: center;
    gap: var(--kh-space-2);
    min-width: 0;
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
  }

  .entry-sub .dot { color: var(--kh-text-subtle); }

  .entry-sub .snip {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Multi-account services: a tidy container with flat rows inside. */
  .group {
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    overflow: hidden;
  }

  /* The header reuses .entry-row layout but drops the standalone card chrome,
     since the .group provides the surrounding card. */
  .group-header {
    border: none;
    border-radius: 0;
  }

  .group-header:hover {
    border-color: transparent;
    box-shadow: none;
    background: var(--kh-surface-sunken);
  }

  .group-header:focus-visible {
    border-color: transparent;
    box-shadow: inset 0 0 0 2px var(--kh-accent-subtle);
  }

  .chevron {
    flex: 0 0 auto;
    width: 18px;
    height: 18px;
    color: var(--kh-text-muted);
    transform: rotate(-90deg); /* collapsed by default → points right */
    transition: transform var(--kh-motion-fast) var(--kh-ease);
  }

  .chevron.open {
    transform: rotate(0deg);
  }

  /* Flat account rows inside a group (dividers, not nested cards). */
  .accounts {
    list-style: none;
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--kh-border);
  }

  .account-row {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
    width: 100%;
    padding: var(--kh-space-3) var(--kh-space-4) var(--kh-space-3) var(--kh-space-7);
    background: var(--kh-surface);
    border: none;
    border-top: 1px solid var(--kh-border);
    text-align: left;
    color: var(--kh-text);
    cursor: pointer;
    transition: background var(--kh-motion-fast) var(--kh-ease);
  }

  .accounts li:first-child .account-row { border-top: none; }

  .account-row:hover { background: var(--kh-surface-sunken); }

  .account-row:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--kh-accent-subtle);
  }

  .account-body {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .account-title {
    font-weight: var(--kh-font-weight-medium);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .account-snippet {
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .arrow {
    flex: 0 0 auto;
    width: 16px;
    height: 16px;
    color: var(--kh-text-subtle);
  }
</style>
