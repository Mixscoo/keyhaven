import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  session,
  vaultPath,
  initSession,
  enterUnlocked,
  lock,
  listenForLock,
} from "./session";
import { entries, entriesTotal } from "./entries";
import { searchQuery } from "./searchQuery";
import { route } from "./navigation";
import { settings } from "./settings";
import { invoke, listen } from "../../test/tauri-mocks";
import type { Settings, VaultSummary } from "../types";

/*
 * session store tests (Req 3.1, 3.4, 3.5, 4.2 — session state transitions).
 *
 * The backend is the source of truth for the unlocked state; this store mirrors
 * it and drives top-level routing. We verify startup routing, the unlock
 * transition, manual lock, and the `vault-locked` event teardown that clears all
 * decrypted in-memory state.
 */

const sampleSettings: Settings = {
  auto_lock_seconds: 300,
  lock_on_blur: false,
  clipboard_clear_seconds: 20,
  password_gen_defaults: {
    length: 20,
    upper: true,
    lower: true,
    digits: true,
    symbols: true,
  },
};

const summary: VaultSummary = {
  hasRecovery: true,
  entryCount: 3,
  unlockedViaRecovery: false,
};

/** Route invoke by command, defaulting unknowns so loadSettings etc. resolve. */
function routeInvoke(overrides: Record<string, unknown> = {}) {
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd in overrides) return overrides[cmd];
    switch (cmd) {
      case "get_settings":
        return sampleSettings;
      case "lock_vault":
        return undefined;
      default:
        return undefined;
    }
  });
}

beforeEach(() => {
  // Reset shared singletons so transitions are observed from a known baseline.
  session.set({ status: "no-vault" });
  vaultPath.set("");
  entries.set([]);
  entriesTotal.set(0);
  searchQuery.set("");
  route.set({ name: "list" });
  routeInvoke();
});

describe("initSession", () => {
  it("routes to unlocked and loads settings when the backend is unlocked", async () => {
    routeInvoke({ is_unlocked: true, get_settings: sampleSettings });

    await initSession();

    expect(get(session).status).toBe("unlocked");
  });

  it("routes to locked when a vault file exists but is not unlocked (Req 3.1)", async () => {
    routeInvoke({ is_unlocked: false, vault_exists: true });

    await initSession();

    expect(get(session).status).toBe("locked");
  });

  it("routes to no-vault when no vault file exists", async () => {
    routeInvoke({ is_unlocked: false, vault_exists: false });

    await initSession();

    expect(get(session).status).toBe("no-vault");
  });
});

describe("enterUnlocked", () => {
  it("transitions to unlocked, stores the summary, and resets navigation", async () => {
    route.set({ name: "editor", entryId: "x" });

    await enterUnlocked(summary);

    const state = get(session);
    expect(state.status).toBe("unlocked");
    expect(state.summary).toEqual(summary);
    expect(get(route)).toEqual({ name: "list" });
  });
});

describe("lock", () => {
  it("asks the backend to lock and clears decrypted in-memory state (Req 3.5)", async () => {
    session.set({ status: "unlocked", summary });
    entries.set([
      { id: "1", serviceRef: { kind: "catalog", id: "github" }, title: "GitHub" },
    ]);
    entriesTotal.set(1);
    searchQuery.set("git");
    settings.set(sampleSettings);

    await lock();

    expect(invoke).toHaveBeenCalledWith("lock_vault");
    expect(get(session).status).toBe("locked");
    expect(get(entries)).toEqual([]);
    expect(get(entriesTotal)).toBe(0);
    expect(get(searchQuery)).toBe("");
  });
});

describe("listenForLock", () => {
  it("subscribes to vault-locked and clears state when the event fires (Req 4.2)", async () => {
    let handler: (() => void) | undefined;
    listen.mockImplementation((event: string, cb: () => void) => {
      if (event === "vault-locked") handler = cb;
      return Promise.resolve(() => {});
    });

    session.set({ status: "unlocked", summary });
    entries.set([
      { id: "1", serviceRef: { kind: "catalog", id: "github" }, title: "GitHub" },
    ]);

    await listenForLock();
    expect(listen).toHaveBeenCalledWith("vault-locked", expect.any(Function));
    expect(handler).toBeDefined();

    // Simulate the backend emitting the lock event (auto-lock / lock-on-blur).
    handler?.();

    expect(get(session).status).toBe("locked");
    expect(get(entries)).toEqual([]);
  });
});
