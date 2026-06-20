<script lang="ts">
  /*
   * A single row in the vault list. Presentational only: it receives a
   * lightweight, non-secret `EntrySummary` (never a secret value) and surfaces
   * an `onOpen` callback when activated.
   *
   * The entry is made identifiable by its service (Req 8.1): a bundled brand
   * logo when the service is in the catalog, otherwise a calm monogram avatar.
   */
  import { onMount } from "svelte";
  import type { EntrySummary } from "../types";
  import ServiceIcon from "./ServiceIcon.svelte";
  import { catalogById, ensureCatalogLoaded } from "../stores/catalog";

  let {
    entry,
    onOpen,
  }: {
    entry: EntrySummary;
    onOpen?: (id: string) => void;
  } = $props();

  onMount(() => {
    if (entry.serviceRef.kind === "catalog") void ensureCatalogLoaded();
  });

  /** Turn a catalog id like `ebay-classifieds` into `Ebay Classifieds`. */
  function prettify(id: string): string {
    return id
      .replace(/[-_]+/g, " ")
      .replace(/\b\w/g, (c) => c.toUpperCase())
      .trim();
  }

  const serviceLabel = $derived(
    entry.serviceRef.kind === "catalog"
      ? prettify(entry.serviceRef.id)
      : "Custom service",
  );
  const primary = $derived(
    entry.title && entry.title.trim().length > 0
      ? entry.title.trim()
      : serviceLabel,
  );
  // The catalog entry (with its logo) for this service, when known.
  const catalogSvc = $derived(
    entry.serviceRef.kind === "catalog"
      ? $catalogById[entry.serviceRef.id]
      : undefined,
  );
</script>

<button
  class="entry-card"
  type="button"
  title={primary}
  onclick={() => onOpen?.(entry.id)}
>
  <ServiceIcon
    name={primary}
    id={entry.serviceRef.id}
    svg={catalogSvc?.svg ?? ""}
    color={catalogSvc?.color ?? ""}
    iconData={catalogSvc?.icon_data ?? ""}
    size={40}
  />
  <span class="body">
    <span class="primary">{primary}</span>
    <span class="meta">
      <span class="service">{serviceLabel}</span>
      {#if entry.snippet}
        <span class="dot" aria-hidden="true">·</span>
        <span class="snippet">{entry.snippet}</span>
      {/if}
    </span>
  </span>
</button>

<style>
  .entry-card {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
    width: 100%;
    height: 100%;
    padding: var(--kh-space-2) var(--kh-space-3);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    text-align: left;
    color: var(--kh-text);
    transition:
      border-color var(--kh-motion-fast) var(--kh-ease),
      box-shadow var(--kh-motion-fast) var(--kh-ease),
      background var(--kh-motion-fast) var(--kh-ease);
  }

  .entry-card:hover {
    border-color: var(--kh-border-strong);
    box-shadow: var(--kh-shadow-sm);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .primary {
    font-weight: var(--kh-font-weight-medium);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: var(--kh-space-2);
    min-width: 0;
    font-size: var(--kh-font-size-sm);
    color: var(--kh-text-muted);
  }

  .service {
    flex: 0 0 auto;
  }

  .dot {
    color: var(--kh-text-subtle);
  }

  .snippet {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
