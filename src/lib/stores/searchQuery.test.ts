import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { get } from "svelte/store";
import {
  searchQuery,
  debouncedSearchQuery,
  SEARCH_DEBOUNCE_MS,
} from "./searchQuery";

/*
 * searchQuery store tests (Req 9.2 — debounced search).
 *
 * The debounced mirror should only settle to the latest typed value after the
 * user pauses for SEARCH_DEBOUNCE_MS, collapsing a burst of keystrokes into a
 * single settled value so the backend isn't queried per keypress.
 */

beforeEach(() => {
  vi.useFakeTimers();
  searchQuery.set("");
});

afterEach(() => {
  vi.useRealTimers();
});

describe("debouncedSearchQuery", () => {
  it("does not emit the new value before the debounce window elapses", () => {
    const seen: string[] = [];
    const unsub = debouncedSearchQuery.subscribe((v) => seen.push(v));

    searchQuery.set("a");
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS - 1);

    // Still only the initial empty value — the typed value hasn't settled yet.
    expect(seen[seen.length - 1]).toBe("");
    unsub();
  });

  it("emits the latest value once after the debounce window (Req 9.2)", () => {
    const seen: string[] = [];
    const unsub = debouncedSearchQuery.subscribe((v) => seen.push(v));

    searchQuery.set("g");
    searchQuery.set("gi");
    searchQuery.set("git");
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);

    expect(get(debouncedSearchQuery)).toBe("git");
    // Only the final settled value is appended beyond the initial "".
    expect(seen.filter((v) => v !== "")).toEqual(["git"]);
    unsub();
  });

  it("collapses a burst of rapid keystrokes into a single settled emission", () => {
    const seen: string[] = [];
    const unsub = debouncedSearchQuery.subscribe((v) => seen.push(v));

    // Type several characters faster than the debounce window each time.
    for (const value of ["n", "ne", "net", "netf", "netfl"]) {
      searchQuery.set(value);
      vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS - 10);
    }
    // Now pause.
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);

    // A fresh subscriber first receives the store's retained value (replayed
    // from a prior settle); ignore that initial emission. Everything after it
    // must collapse to the single final value — no per-keystroke emissions.
    const afterInitial = seen.slice(1);
    expect(afterInitial).toEqual(["netfl"]);
    unsub();
  });

  it("emits each value when the user pauses between entries", () => {
    const seen: string[] = [];
    const unsub = debouncedSearchQuery.subscribe((v) => seen.push(v));

    searchQuery.set("aws");
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);
    searchQuery.set("");
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);

    expect(seen).toContain("aws");
    expect(get(debouncedSearchQuery)).toBe("");
    unsub();
  });
});
