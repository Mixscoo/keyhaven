<script lang="ts" generics="T">
  /*
   * Lightweight windowed (virtualized) list.
   *
   * Only the rows currently in (or near) the viewport are mounted, so the list
   * stays smooth with thousands of items (Req 9.3) without pulling in a heavy
   * dependency. Rows are assumed to be a fixed `itemHeight`; a tall spacer
   * reserves the full scroll height and the visible window is translated into
   * place.
   *
   * `onEndReached` fires when the user scrolls near the bottom, enabling
   * incremental ("load more") paging on top of windowing.
   */
  import type { Snippet } from "svelte";

  let {
    items,
    itemHeight,
    overscan = 6,
    onEndReached,
    endThreshold = 240,
    key,
    row,
  }: {
    items: T[];
    /** Fixed pixel height of every row. */
    itemHeight: number;
    /** Extra rows rendered above/below the viewport to mask fast scrolling. */
    overscan?: number;
    /** Called once when the scroll position nears the bottom. */
    onEndReached?: () => void;
    /** Distance (px) from the bottom at which `onEndReached` fires. */
    endThreshold?: number;
    /** Stable key per item so DOM nodes aren't reused across different data. */
    key?: (item: T, index: number) => string | number;
    /** Renders a single row. Receives the item and its absolute index. */
    row: Snippet<[T, number]>;
  } = $props();

  let viewport = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let viewportHeight = $state(0);

  const count = $derived(items.length);
  const totalHeight = $derived(count * itemHeight);
  const startIndex = $derived(
    Math.max(0, Math.floor(scrollTop / itemHeight) - overscan),
  );
  const endIndex = $derived(
    Math.min(count, Math.ceil((scrollTop + viewportHeight) / itemHeight) + overscan),
  );
  const visible = $derived(items.slice(startIndex, endIndex));
  const offsetY = $derived(startIndex * itemHeight);

  function handleScroll() {
    const el = viewport;
    if (!el) return;
    scrollTop = el.scrollTop;
    if (
      onEndReached &&
      el.scrollTop + el.clientHeight >= totalHeight - endThreshold
    ) {
      onEndReached();
    }
  }

  // Track the viewport height so the visible window adapts to resizes.
  $effect(() => {
    const el = viewport;
    if (!el) return;
    viewportHeight = el.clientHeight;
    const ro = new ResizeObserver((entries) => {
      viewportHeight = entries[0]?.contentRect.height ?? el.clientHeight;
    });
    ro.observe(el);
    return () => ro.disconnect();
  });
</script>

<div class="viewport" bind:this={viewport} onscroll={handleScroll}>
  <div class="sizer" style="height: {totalHeight}px">
    <div class="window" style="transform: translateY({offsetY}px)">
      {#each visible as item, i (key ? key(item, startIndex + i) : startIndex + i)}
        <div class="row" style="height: {itemHeight}px">
          {@render row(item, startIndex + i)}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .viewport {
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    /* Smooth, contained scrolling; isolates layout/paint for long lists. */
    contain: strict;
  }

  .sizer {
    position: relative;
    width: 100%;
  }

  .window {
    position: absolute;
    inset: 0 0 auto 0;
    will-change: transform;
  }

  .row {
    box-sizing: border-box;
  }
</style>
