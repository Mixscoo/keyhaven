import { readable, writable } from "svelte/store";

// Raw search text bound to the search input. Updated on every keystroke.
export const searchQuery = writable<string>("");

// Debounce window (design: ~120ms) so we don't fire a backend query per keypress.
export const SEARCH_DEBOUNCE_MS = 120;

/**
 * Debounced mirror of {@link searchQuery} that drives `list_entries`. It settles
 * to the latest value only after the user pauses typing for
 * {@link SEARCH_DEBOUNCE_MS}, keeping the entry list responsive at scale without
 * a request on every keystroke (Req 9.2).
 */
export const debouncedSearchQuery = readable<string>("", (set) => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const unsubscribe = searchQuery.subscribe((value) => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => set(value), SEARCH_DEBOUNCE_MS);
  });
  return () => {
    if (timer) clearTimeout(timer);
    unsubscribe();
  };
});
