import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import EntryEditor from "./EntryEditor.svelte";
import { route } from "../lib/stores/navigation";
import { settings } from "../lib/stores/settings";
import { invoke } from "../test/tauri-mocks";
import type { Entry, Settings } from "../lib/types";

/*
 * EntryEditor tests (Req 7.2 add fields, 7.3 remove fields, 7.4 secret masking
 * with reveal).
 *
 * Existing entries open in a read-only View (the route carries an entryId);
 * fields are visible with mask/reveal but not editable until the user clicks
 * "Edit". Tests that change fields enter edit mode first via `enterEditMode`.
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

const entry: Entry = {
  id: "e1",
  service_ref: { kind: "catalog", id: "github" },
  title: "GitHub",
  fields: [
    {
      id: "f1",
      label: "Email",
      type: "email",
      value: "me@example.com",
      secret: false,
    },
    {
      id: "f2",
      label: "Password",
      type: "password",
      value: "s3cret-value",
      secret: true,
    },
  ],
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
};

function routeInvoke() {
  invoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "get_entry":
        return structuredClone(entry);
      case "report_activity":
        return undefined;
      case "generate_password":
        return "generated-pw";
      default:
        return undefined;
    }
  });
}

beforeEach(() => {
  settings.set(sampleSettings);
  route.set({ name: "editor", entryId: "e1" });
  routeInvoke();
});

/** All field value inputs currently rendered, in order. */
function valueInputs(): HTMLInputElement[] {
  return screen.getAllByLabelText("Field value") as HTMLInputElement[];
}

/** Switch from the read-only View into edit mode. */
async function enterEditMode(): Promise<void> {
  await fireEvent.click(screen.getByRole("button", { name: "Edit" }));
}

describe("EntryEditor field editing", () => {
  it("loads an existing entry's fields (read-only View)", async () => {
    render(EntryEditor);

    await waitFor(() => expect(valueInputs()).toHaveLength(2));
    // In View mode the label is a caption, not an editable field.
    expect(screen.getByText("Email")).toBeInTheDocument();
  });

  it("masks a secret field by default and reveals it on toggle (Req 7.4)", async () => {
    render(EntryEditor);
    await waitFor(() => expect(valueInputs()).toHaveLength(2));

    // The secret password field is masked (type=password) by default.
    const secretInput = valueInputs()[1];
    expect(secretInput.type).toBe("password");
    expect(secretInput.value).toBe("s3cret-value");

    // Reveal it.
    await fireEvent.click(screen.getByRole("button", { name: "Reveal value" }));
    await waitFor(() => {
      expect(valueInputs()[1].type).toBe("text");
    });

    // And hide it again.
    await fireEvent.click(screen.getByRole("button", { name: "Hide value" }));
    await waitFor(() => {
      expect(valueInputs()[1].type).toBe("password");
    });
  });

  it("leaves a non-secret field unmasked (Req 7.4)", async () => {
    render(EntryEditor);
    await waitFor(() => expect(valueInputs()).toHaveLength(2));

    expect(valueInputs()[0].type).toBe("text");
  });

  it("adds a new empty field when 'Add field' is clicked (Req 7.2)", async () => {
    render(EntryEditor);
    await waitFor(() => expect(valueInputs()).toHaveLength(2));
    await enterEditMode();

    await fireEvent.click(screen.getByRole("button", { name: /Add field/ }));

    await waitFor(() => expect(valueInputs()).toHaveLength(3));
    // The newly added field starts empty.
    expect(valueInputs()[2].value).toBe("");
  });

  it("removes a field when its remove control is clicked (Req 7.3)", async () => {
    render(EntryEditor);
    await waitFor(() => expect(valueInputs()).toHaveLength(2));
    await enterEditMode();

    const removeButtons = screen.getAllByLabelText("Remove field");
    await fireEvent.click(removeButtons[0]);

    await waitFor(() => expect(valueInputs()).toHaveLength(1));
    // The remaining field is the former second one (the password field).
    expect(valueInputs()[0].value).toBe("s3cret-value");
  });

  it("can add then remove a field, returning to the original count (Req 7.2, 7.3)", async () => {
    render(EntryEditor);
    await waitFor(() => expect(valueInputs()).toHaveLength(2));
    await enterEditMode();

    await fireEvent.click(screen.getByRole("button", { name: /Add field/ }));
    await waitFor(() => expect(valueInputs()).toHaveLength(3));

    const removeButtons = screen.getAllByLabelText("Remove field");
    await fireEvent.click(removeButtons[removeButtons.length - 1]);

    await waitFor(() => expect(valueInputs()).toHaveLength(2));
  });
});
