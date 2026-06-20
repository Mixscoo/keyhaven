import { writable } from "svelte/store";
import type { ToastMessage } from "../types";

// How long a toast stays before auto-dismissing (ms). 0 = sticky.
const DEFAULT_TIMEOUT_MS = 4000;

function createToastStore() {
  const { subscribe, update } = writable<ToastMessage[]>([]);
  let nextId = 1;

  function dismiss(id: number) {
    update((list) => list.filter((t) => t.id !== id));
  }

  return {
    subscribe,
    /**
     * Show a transient notification. Auto-dismisses after `timeout` ms unless
     * `timeout` is 0. Returns the id so callers can dismiss it early.
     */
    push(kind: ToastMessage["kind"], text: string, timeout = DEFAULT_TIMEOUT_MS) {
      const id = nextId++;
      update((list) => [...list, { id, kind, text }]);
      if (timeout > 0) {
        setTimeout(() => dismiss(id), timeout);
      }
      return id;
    },
    dismiss,
    /** Remove all toasts (e.g. on lock). */
    clear() {
      update(() => []);
    },
  };
}

// Transient notifications.
export const toast = createToastStore();
