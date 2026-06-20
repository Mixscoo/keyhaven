import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import ServicePicker from "./ServicePicker.svelte";
import { invoke } from "../../test/tauri-mocks";
import type { CatalogService, CustomService, IconRef } from "../types";

/*
 * ServicePicker tests.
 *
 * Covers the catalog search surface, rendering of the user's custom services,
 * the "create custom service" flow, and that a selection emits a ServiceSelection
 * carrying the recommended fields the EntryEditor uses to prefill (Req 5.1, 5.2,
 * 6.1, 6.3, 7.1).
 */

const githubService: CatalogService = {
  id: "github",
  name: "GitHub",
  icon: "github.svg",
  aliases: ["gh"],
  recommended_fields: [
    { label: "Email", type: "email", secret: false },
    { label: "Password", type: "password", secret: true },
  ],
};

const googleService: CatalogService = {
  id: "google",
  name: "Google",
  icon: "google.svg",
  aliases: [],
  recommended_fields: [{ label: "Email", type: "email", secret: false }],
};

const customService: CustomService = {
  id: "custom-1",
  name: "Home Server",
  icon: { kind: "builtin", ref: "" },
  custom: true,
};

/** Configure invoke to serve catalog/custom-service data, filtering by query. */
function setupBackend(options: {
  catalog?: CatalogService[];
  custom?: CustomService[];
} = {}) {
  const catalog = options.catalog ?? [githubService, googleService];
  const custom = options.custom ?? [];
  invoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case "search_catalog": {
        const q = ((args?.query as string) ?? "").toLowerCase().trim();
        if (!q) return catalog;
        return catalog.filter(
          (s) =>
            s.name.toLowerCase().includes(q) ||
            s.aliases.some((a) => a.toLowerCase().includes(q)),
        );
      }
      case "list_custom_services":
        return custom;
      case "create_custom_service": {
        return {
          id: "custom-new",
          name: args.name as string,
          icon: args.icon as IconRef,
          custom: true,
        } satisfies CustomService;
      }
      default:
        return undefined;
    }
  });
}

beforeEach(() => {
  setupBackend();
});

describe("ServicePicker", () => {
  it("lists catalog services from an empty (default) search", async () => {
    render(ServicePicker, { props: { onSelect: vi.fn() } });

    expect(await screen.findByText("GitHub")).toBeInTheDocument();
    expect(screen.getByText("Google")).toBeInTheDocument();
  });

  it("debounces the search and filters the catalog (Req 5.1)", async () => {
    render(ServicePicker, { props: { onSelect: vi.fn() } });
    await screen.findByText("GitHub");

    const search = screen.getByLabelText("Search the service catalog");
    await fireEvent.input(search, { target: { value: "google" } });

    await waitFor(() => {
      expect(screen.queryByText("GitHub")).not.toBeInTheDocument();
    });
    expect(screen.getByText("Google")).toBeInTheDocument();

    // The debounce should collapse to a single trailing search for the query,
    // not one request per keystroke.
    const googleSearches = invoke.mock.calls.filter(
      (c) => c[0] === "search_catalog" && (c[1] as { query?: string })?.query === "google",
    );
    expect(googleSearches.length).toBe(1);
  });

  it("emits a catalog selection with recommended fields (Req 5.2, 7.1)", async () => {
    const onSelect = vi.fn();
    render(ServicePicker, { props: { onSelect } });
    await screen.findByText("GitHub");

    await fireEvent.click(screen.getByText("GitHub"));

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith({
      serviceRef: { kind: "catalog", id: "github" },
      name: "GitHub",
      custom: false,
      recommendedFields: githubService.recommended_fields,
    });
  });

  it("renders the user's custom services with a Custom badge (Req 6.3)", async () => {
    setupBackend({ custom: [customService] });
    const onSelect = vi.fn();
    render(ServicePicker, { props: { onSelect } });

    expect(await screen.findByText("Home Server")).toBeInTheDocument();
    expect(screen.getByText("Custom")).toBeInTheDocument();

    await fireEvent.click(screen.getByText("Home Server"));
    expect(onSelect).toHaveBeenCalledWith({
      serviceRef: { kind: "custom", id: "custom-1" },
      name: "Home Server",
      custom: true,
      recommendedFields: [],
    });
  });

  it("creates a custom service and selects it (Req 6.1, 6.2)", async () => {
    const onSelect = vi.fn();
    render(ServicePicker, { props: { onSelect } });
    await screen.findByText("GitHub");

    await fireEvent.click(
      screen.getByRole("button", { name: /Create custom service/ }),
    );

    const nameInput = screen.getByLabelText("Service name");
    await fireEvent.input(nameInput, { target: { value: "My VPN" } });

    await fireEvent.click(
      screen.getByRole("button", { name: /Create and use/ }),
    );

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(
        "create_custom_service",
        expect.objectContaining({ name: "My VPN" }),
      );
    });
    expect(onSelect).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "My VPN",
        custom: true,
        recommendedFields: [],
      }),
    );
  });
});
