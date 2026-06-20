<script lang="ts">
  // Application shell: top-level route gating based on session status.
  //   no-vault → Setup   |   locked → Unlock   |   unlocked → Vault
  // The backend is the source of truth for the session status; this shell also
  // listens for the `vault-locked` event (auto-lock timeout / lock-on-blur /
  // explicit lock) and routes back to Unlock, clearing in-memory decrypted
  // state. Screen internals (Setup/Unlock/Vault/etc.) arrive in tasks 10–13.
  import { onMount } from "svelte";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { session, initSession, listenForLock } from "./lib/stores/session";
  import { route } from "./lib/stores/navigation";
  import { toast } from "./lib/stores/toast";
  import Setup from "./views/Setup.svelte";
  import Unlock from "./views/Unlock.svelte";
  import Vault from "./views/Vault.svelte";
  import EntryEditor from "./views/EntryEditor.svelte";
  import Settings from "./views/Settings.svelte";
  import Toast from "./lib/components/Toast.svelte";

  // Gate rendering until the initial session status is known so we don't flash
  // the wrong screen.
  let ready = $state(false);

  onMount(() => {
    let unlisten: UnlistenFn | undefined;

    (async () => {
      try {
        await initSession();
      } catch {
        // If status can't be determined, fall back to the first-run screen.
        toast.push("danger", "Could not determine vault status.");
      } finally {
        ready = true;
      }

      try {
        unlisten = await listenForLock();
      } catch {
        toast.push("warning", "Auto-lock notifications are unavailable.");
      }
    })();

    return () => unlisten?.();
  });
</script>

{#if ready}
  {#if $session.status === "no-vault"}
    <Setup />
  {:else if $session.status === "locked"}
    <Unlock />
  {:else if $route.name === "editor"}
    <EntryEditor />
  {:else if $route.name === "settings"}
    <Settings />
  {:else}
    <Vault />
  {/if}
{:else}
  <main class="booting" aria-busy="true">
    <p>Loading Keyhaven…</p>
  </main>
{/if}

<Toast />

<style>
  .booting {
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--kh-text-muted);
  }
</style>
