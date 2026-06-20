<script lang="ts">
  // Renders transient notifications from the toast store. Styling refined later.
  import { toast } from "../stores/toast";
</script>

<div class="toast-region" aria-live="polite">
  {#each $toast as t (t.id)}
    <div class="toast {t.kind}">
      <span>{t.text}</span>
      <button type="button" aria-label="Dismiss" onclick={() => toast.dismiss(t.id)}>×</button>
    </div>
  {/each}
</div>

<style>
  .toast-region {
    position: fixed;
    bottom: var(--kh-space-4);
    right: var(--kh-space-4);
    display: flex;
    flex-direction: column;
    gap: var(--kh-space-2);
    z-index: 1000;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: var(--kh-space-3);
    padding: var(--kh-space-3) var(--kh-space-4);
    background: var(--kh-surface);
    border: 1px solid var(--kh-border);
    border-radius: var(--kh-radius);
    box-shadow: var(--kh-shadow);
  }
  button {
    border: none;
    background: transparent;
    cursor: pointer;
    font-size: 1.1rem;
    color: var(--kh-text-muted);
  }
</style>
