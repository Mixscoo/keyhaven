<script lang="ts">
  /*
   * Main vault view: a search bar over a virtualized, service-identifiable
   * entry list, with a prominent "Add" action and a manual "Lock" control.
   *
   * Data flow:
   *   - The search input is bound to the `searchQuery` store; a debounced mirror
   *     (`debouncedSearchQuery`, ~120ms) drives backend `list_entries` calls so
   *     results update responsively without a request per keystroke (Req 9.2).
   *   - The list renders only the rows in view (VirtualList) and pages more in
   *     as the user scrolls, keeping thousands of entries smooth (Req 9.3).
   *   - Each row is identifiable by its service (Req 8.1) via EntryCard.
   *   - "Lock" calls the backend lock (which zeroizes the VEK/model and emits
   *     `vault-locked`); the shell routes back to Unlock (Req 3.5).
   */
  import { get } from "svelte/store";
  import {
    entries,
    entriesTotal,
    loadEntries,
    loadMoreEntries,
  } from "../lib/stores/entries";
  import { searchQuery, debouncedSearchQuery } from "../lib/stores/searchQuery";
  import { lock } from "../lib/stores/session";
  import { openEditor, openSettings } from "../lib/stores/navigation";
  import { toast } from "../lib/stores/toast";
  import { reportActivity } from "../lib/api";
  import VirtualList from "../lib/components/VirtualList.svelte";
  import EntryCard from "../lib/components/EntryCard.svelte";
  import type { EntrySummary } from "../lib/types";

  // Fixed row height (px) the virtualized list lays out against. Must match the
  // EntryCard footprint (40px avatar + padding) plus the inter-row gap below.
  const ROW_HEIGHT = 72;

  let loading = $state(false);
  let loadingMore = $state(false);
  let locking = $state(false);
  // The query the currently displayed list was loaded with; used so paging and
  // refreshes stay consistent with what the user last searched.
  let activeQuery = $state("");

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

  async function loadMore(): Promise<void> {
    if (loadingMore || loading) return;
    if (get(entries).length >= get(entriesTotal)) return;
    loadingMore = true;
    try {
      await loadMoreEntries(activeQuery);
    } catch {
      toast.push("danger", "Could not load more entries.");
    } finally {
      loadingMore = false;
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

  // React to debounced search changes (and the initial empty query) by
  // reloading the list. `debouncedSearchQuery` settles to "" on mount, which
  // performs the first load.
  let lastLoaded = $state<string | null>(null);
  $effect(() => {
    const query = $debouncedSearchQuery;
    if (query === lastLoaded) return;
    lastLoaded = query;
    pingActivity();
    void refresh(query);
  });

  const entryKey = (entry: EntrySummary): string => entry.id;
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
        class="btn ghost"
        type="button"
        onclick={handleLock}
        disabled={locking}
      >
        Lock
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
      {$entriesTotal === 1 ? "entry" : "entries"}
      {activeQuery.length > 0 ? "found" : ""}
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
      <VirtualList
        items={$entries}
        itemHeight={ROW_HEIGHT}
        key={entryKey}
        onEndReached={loadMore}
      >
        {#snippet row(entry: EntrySummary)}
          <div class="row-inner">
            <EntryCard {entry} onOpen={handleOpen} />
          </div>
        {/snippet}
      </VirtualList>
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

  /* Square, centered icon-only button so the settings cog reads as an icon
     control rather than a text button. */
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

  /* Vertical gap between rows without breaking the fixed-height windowing:
     the row container is ROW_HEIGHT; the inner card insets to leave a gap. */
  .row-inner {
    height: 100%;
    padding-bottom: var(--kh-space-2);
  }
</style>
