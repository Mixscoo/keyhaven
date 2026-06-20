import { get, writable } from "svelte/store";
import type { EntrySummary } from "../types";
import * as api from "../api";

// Default page size; mirrors the backend's DEFAULT_PAGE_LIMIT.
export const DEFAULT_PAGE_LIMIT = 100;

// Current page/filtered list. Kept lightweight: the list view uses EntrySummary;
// full secrets are fetched on demand (via api.getEntry) when an entry is opened.
export const entries = writable<EntrySummary[]>([]);

// Pagination state: total matches (pre-pagination) and the applied window.
export const entriesTotal = writable<number>(0);
export const entriesOffset = writable<number>(0);
export const entriesLimit = writable<number>(DEFAULT_PAGE_LIMIT);

/**
 * Load the first page of entry summaries for `query`, replacing the list.
 * Resets the offset to 0 so a new query (or refresh) always starts from the top.
 * Requires an unlocked session (the backend returns `Locked` otherwise).
 */
export async function loadEntries(query?: string): Promise<void> {
  entriesOffset.set(0);
  const result = await api.listEntries(query, {
    offset: 0,
    limit: get(entriesLimit),
  });
  entries.set(result.entries);
  entriesTotal.set(result.total);
  entriesOffset.set(result.offset);
  entriesLimit.set(result.limit);
}

/**
 * Append the next page of summaries for `query` (incremental paging behind the
 * virtualized list). No-op once every matching entry is loaded. Pass the SAME
 * query used for {@link loadEntries} so pagination stays consistent.
 */
export async function loadMoreEntries(query?: string): Promise<void> {
  const loaded = get(entries).length;
  const total = get(entriesTotal);
  // Nothing more to fetch (total === 0 only before the first load completes).
  if (total !== 0 && loaded >= total) return;

  const result = await api.listEntries(query, {
    offset: loaded,
    limit: get(entriesLimit),
  });
  entries.update((current) => [...current, ...result.entries]);
  entriesTotal.set(result.total);
  entriesOffset.set(result.offset);
}

/** Clear all in-memory list state. Used on lock so no decrypted data lingers. */
export function resetEntries(): void {
  entries.set([]);
  entriesTotal.set(0);
  entriesOffset.set(0);
  entriesLimit.set(DEFAULT_PAGE_LIMIT);
}
