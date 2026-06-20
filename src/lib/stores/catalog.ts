/*
 * A tiny, lazily-populated cache of the bundled service catalog, keyed by
 * catalog id. The list view (EntryCard) and the editor need a service's logo
 * given only its `serviceRef.id`, so we load the whole catalog once (it's small
 * and fully local) and expose an id → service map as a store.
 */
import { writable } from "svelte/store";
import * as api from "../api";
import type { CatalogService } from "../types";

/** id → catalog service. Empty until {@link ensureCatalogLoaded} resolves. */
export const catalogById = writable<Record<string, CatalogService>>({});

let started = false;

/** Load the catalog once and populate {@link catalogById}. Safe to call often. */
export async function ensureCatalogLoaded(): Promise<void> {
  if (started) return;
  started = true;
  try {
    const list = await api.searchCatalog("");
    const map: Record<string, CatalogService> = {};
    for (const service of list) map[service.id] = service;
    catalogById.set(map);
  } catch {
    // Allow a later retry if the first load failed.
    started = false;
  }
}
