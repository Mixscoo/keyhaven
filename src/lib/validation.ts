/*
 * Lightweight, offline field-value validation for the entry editor.
 *
 * The backend stores arbitrary strings, so this is a UX guard: it stops obvious
 * mistakes (a name typed into an email field, letters in a phone number, a
 * malformed URL) before an entry is saved. Only format-bearing types are
 * checked; free-form types (text, note, password, username, secrets) accept
 * anything. An empty value is never an error here — emptiness is handled at the
 * form level (an entry must have at least one filled field).
 */
import type { FieldType } from "./types";

// Pragmatic email shape: something@something.tld with no spaces.
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/;
// Allowed phone characters; digit count is checked separately.
const PHONE_SHAPE_RE = /^[+()\d][\d\s().-]*$/;

/**
 * Validate a field's value for its type. Returns a human-readable error string,
 * or `null` when the value is acceptable (including when it is empty).
 */
export function validateFieldValue(type: FieldType, rawValue: string): string | null {
  const value = rawValue.trim();
  if (value.length === 0) return null;

  switch (type) {
    case "email":
      return EMAIL_RE.test(value)
        ? null
        : "Enter a valid email address (e.g. name@example.com).";

    case "phone": {
      const digits = value.replace(/\D/g, "");
      return PHONE_SHAPE_RE.test(value) && digits.length >= 6
        ? null
        : "Enter a valid phone number (digits, spaces, and + - ( ) only).";
    }

    case "url":
      return isUrlLike(value)
        ? null
        : "Enter a valid web address (e.g. https://example.com).";

    default:
      return null;
  }
}

/** Whether `value` looks like a usable web address (scheme optional). */
function isUrlLike(value: string): boolean {
  const candidate = /^[a-z][a-z0-9+.-]*:\/\//i.test(value)
    ? value
    : `https://${value}`;
  try {
    const url = new URL(candidate);
    return url.hostname.includes(".") || url.hostname === "localhost";
  } catch {
    return false;
  }
}
