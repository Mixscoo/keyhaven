import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import PasswordGenerator from "./PasswordGenerator.svelte";
import { invoke } from "../../test/tauri-mocks";
import type { PasswordGenOptions } from "../types";

/*
 * PasswordGenerator UI tests (Req 10.2 — configurable length/charset).
 *
 * The component never generates a password itself; it calls the backend
 * `generate_password` command (via api.generatePassword → invoke) with the
 * current options. We assert the control surface forwards the right options and
 * surfaces results, not the randomness itself (that's backend-tested).
 */

const defaults: PasswordGenOptions = {
  length: 20,
  upper: true,
  lower: true,
  digits: true,
  symbols: true,
};

/** Read the `opts` argument from the latest generate_password invoke call. */
function lastGenOpts(): PasswordGenOptions | undefined {
  const calls = invoke.mock.calls.filter((c) => c[0] === "generate_password");
  const last = calls[calls.length - 1];
  return last?.[1]?.opts as PasswordGenOptions | undefined;
}

beforeEach(() => {
  // Echo the requested options into a deterministic, inspectable string so the
  // preview reflects what was asked for.
  invoke.mockImplementation(async (cmd: string, args: { opts: PasswordGenOptions }) => {
    if (cmd === "generate_password") {
      const o = args.opts;
      return `pw-len${o.length}-${o.upper ? "U" : ""}${o.lower ? "l" : ""}${o.digits ? "d" : ""}${o.symbols ? "s" : ""}`;
    }
    return undefined;
  });
});

describe("PasswordGenerator", () => {
  it("generates an initial preview using the seeded defaults on open", async () => {
    render(PasswordGenerator, { props: { initial: defaults, onUse: vi.fn() } });

    await waitFor(() => {
      expect(lastGenOpts()).toEqual(defaults);
    });
    expect(await screen.findByText("pw-len20-Ulds")).toBeInTheDocument();
  });

  it("requests a new length when the length slider changes (Req 10.2)", async () => {
    render(PasswordGenerator, { props: { initial: defaults, onUse: vi.fn() } });
    await waitFor(() => expect(lastGenOpts()).toBeDefined());

    const slider = screen.getByLabelText("Password length") as HTMLInputElement;
    await fireEvent.input(slider, { target: { value: "32" } });

    await waitFor(() => {
      expect(lastGenOpts()?.length).toBe(32);
    });
    expect(screen.getByText(/Length: 32/)).toBeInTheDocument();
  });

  it("toggles charsets and forwards the selection to the backend (Req 10.2)", async () => {
    render(PasswordGenerator, { props: { initial: defaults, onUse: vi.fn() } });
    await waitFor(() => expect(lastGenOpts()).toBeDefined());

    // Turn off symbols.
    const symbols = screen.getByLabelText("!@#") as HTMLInputElement;
    await fireEvent.click(symbols);

    await waitFor(() => {
      expect(lastGenOpts()?.symbols).toBe(false);
    });
    expect(lastGenOpts()).toMatchObject({ upper: true, lower: true, digits: true });
  });

  it("clears the preview and disables actions when no charset is selected", async () => {
    render(PasswordGenerator, { props: { initial: defaults, onUse: vi.fn() } });
    await waitFor(() => expect(lastGenOpts()).toBeDefined());

    for (const label of ["A-Z", "a-z", "0-9", "!@#"]) {
      await fireEvent.click(screen.getByLabelText(label));
    }

    await waitFor(() => {
      expect(
        screen.getByText("Select at least one character set."),
      ).toBeInTheDocument();
    });
    const useBtn = screen.getByRole("button", { name: "Use password" });
    expect(useBtn).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Regenerate password" }),
    ).toBeDisabled();
  });

  it("emits the generated password via onUse when accepted (Req 10.4)", async () => {
    const onUse = vi.fn();
    render(PasswordGenerator, { props: { initial: defaults, onUse } });
    await screen.findByText("pw-len20-Ulds");

    await fireEvent.click(screen.getByRole("button", { name: "Use password" }));

    expect(onUse).toHaveBeenCalledWith("pw-len20-Ulds");
  });

  it("re-requests a password when Regenerate is clicked", async () => {
    render(PasswordGenerator, { props: { initial: defaults, onUse: vi.fn() } });
    await screen.findByText("pw-len20-Ulds");

    const before = invoke.mock.calls.filter(
      (c) => c[0] === "generate_password",
    ).length;
    await fireEvent.click(
      screen.getByRole("button", { name: "Regenerate password" }),
    );

    await waitFor(() => {
      const after = invoke.mock.calls.filter(
        (c) => c[0] === "generate_password",
      ).length;
      expect(after).toBeGreaterThan(before);
    });
  });
});
