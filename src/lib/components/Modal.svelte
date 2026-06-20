<script lang="ts">
  // Placeholder skeleton. Reusable modal; styled fully in later UI tasks.
  import type { Snippet } from "svelte";

  let {
    open = false,
    children,
  }: { open?: boolean; children?: Snippet } = $props();
</script>

{#if open}
  <div class="modal-backdrop" role="presentation">
    <div class="modal" role="dialog" aria-modal="true">
      {@render children?.()}
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(31, 41, 51, 0.25);
  }
  .modal {
    background: var(--kh-surface);
    border-radius: var(--kh-radius-lg);
    box-shadow: var(--kh-shadow);
    padding: var(--kh-space-5);
    min-width: 320px;
  }
</style>
