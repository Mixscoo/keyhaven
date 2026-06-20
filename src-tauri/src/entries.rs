//! Entry repository: create/read/update/delete over the decrypted vault model.
//!
//! This module owns the **in-memory CRUD logic** for [`Entry`] records (task
//! 6.1). It operates purely on a borrowed [`VaultModel`] — it performs no disk
//! I/O and no cryptography. The command layer ([`crate::commands`]) is
//! responsible for gating these operations behind the unlocked session and for
//! persisting the mutated model through the vault's atomic write path
//! ([`crate::vault::OpenVault::save`]).
//!
//! Keeping the CRUD logic free of I/O makes it directly unit-testable and keeps
//! a single, clear seam between "mutate the model" and "persist the model".
//!
//! ## Identifiers & timestamps (Req 5.5, 7.6)
//!
//! - Every entry gets a stable UUID v4 [`Entry::id`] on creation; every field
//!   gets a stable UUID v4 [`Field::id`]. Field ids are **preserved across
//!   edits** when the caller round-trips them (see [`FieldInput::id`]), so a
//!   field keeps its identity through updates; new fields are assigned fresh
//!   ids.
//! - `created_at` is stamped once at creation and never changed afterwards;
//!   `updated_at` is set equal to `created_at` at creation and bumped on every
//!   successful [`update_entry`] (Req 5.5, 8.5).
//!
//! The "current time" is injected as an ISO-8601 string parameter rather than
//! read from the clock here, so the CRUD functions stay pure and deterministic
//! for testing. The command layer supplies [`now_iso`].
//!
//! ## Flexible fields (Req 7.2, 7.3, 7.5)
//!
//! [`EntryInput`] carries an arbitrary list of [`FieldInput`]s, each with a
//! custom label, a [`FieldType`], a value, and a `secret` flag — so callers can
//! add fields, remove fields (by omitting them), relabel them, and toggle
//! secrecy freely. The stored field set is replaced wholesale by what the input
//! describes.
//!
//! ## Empty-entry warning (Req 5.6, 8.6)
//!
//! Saving an entry with nothing filled in is **warned about, not blocked**.
//! [`entry_is_empty`] computes the warning signal; the command layer returns it
//! alongside the saved entry so the UI can surface a non-blocking warning.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{Entry, Field, FieldType, ServiceRef, VaultModel};

/// Errors from entry CRUD operations over the decrypted model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryError {
    /// No entry with the given id exists in the vault.
    NotFound,
}

impl std::fmt::Display for EntryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryError::NotFound => write!(f, "no entry with that id exists"),
        }
    }
}

impl std::error::Error for EntryError {}

/// The caller-supplied shape used to create or update an entry.
///
/// Deserialized from the frontend `EntryInput`. It deliberately omits the
/// server-managed fields (`id`, `created_at`, `updated_at`): those are assigned
/// and maintained by this module so the frontend can never forge or stomp them.
#[derive(Debug, Clone, Deserialize)]
pub struct EntryInput {
    /// Which service this entry is for (catalog or custom).
    pub service_ref: ServiceRef,
    /// Optional user label distinguishing entries for the same service.
    #[serde(default)]
    pub title: Option<String>,
    /// The full, ordered set of fields this entry should have after the
    /// operation. On update this replaces the existing field set wholesale.
    #[serde(default)]
    pub fields: Vec<FieldInput>,
}

/// One field within an [`EntryInput`].
///
/// `id` is optional: when present (an existing field being edited) it is
/// preserved so the field keeps a stable identity across updates; when absent
/// (a newly added field) a fresh UUID is assigned.
#[derive(Debug, Clone, Deserialize)]
pub struct FieldInput {
    /// Existing field id to preserve, or `None` for a newly added field.
    #[serde(default)]
    pub id: Option<String>,
    /// Human-readable, fully custom label.
    pub label: String,
    /// Semantic type of the field. Deserialized from the `type` wire key.
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// The field's value (plaintext within the decrypted model).
    pub value: String,
    /// Whether the value is sensitive (masking / clipboard auto-clear / never
    /// indexed for search).
    pub secret: bool,
}

/// The current wall-clock time as an ISO-8601 / RFC-3339 UTC string with second
/// precision and a trailing `Z` (e.g. `2024-01-31T12:34:56Z`).
///
/// Kept out of the pure CRUD functions so they remain deterministic; the
/// command layer calls this and passes the result in.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Materialize the [`FieldInput`]s into stored [`Field`]s, preserving provided
/// ids and minting UUID v4 ids for fields that lack one.
fn build_fields(inputs: Vec<FieldInput>) -> Vec<Field> {
    inputs
        .into_iter()
        .map(|f| Field {
            id: f.id.unwrap_or_else(new_id),
            label: f.label,
            field_type: f.field_type,
            value: f.value,
            secret: f.secret,
        })
        .collect()
}

/// Generate a fresh UUID v4 identifier string.
fn new_id() -> String {
    Uuid::new_v4().to_string()
}

/// Create a new entry from `input`, append it to `model`, and return a clone of
/// the stored entry (Req 5.3, 5.4, 5.5, 7.2, 7.5).
///
/// A fresh UUID v4 is assigned to the entry and to every field that does not
/// carry one. Both `created_at` and `updated_at` are stamped with `now`.
pub fn create_entry(model: &mut VaultModel, input: EntryInput, now: String) -> Entry {
    let entry = Entry {
        id: new_id(),
        service_ref: input.service_ref,
        title: input.title,
        fields: build_fields(input.fields),
        created_at: now.clone(),
        updated_at: now,
    };
    model.entries.push(entry.clone());
    entry
}

/// Borrow the entry with the given `id`, if present (Req 8.2).
pub fn get_entry<'a>(model: &'a VaultModel, id: &str) -> Option<&'a Entry> {
    model.entries.iter().find(|e| e.id == id)
}

/// Update the entry with the given `id` from `input`, returning a clone of the
/// updated entry (Req 7.3, 7.6, 8.5).
///
/// The entry id and `created_at` are preserved; `updated_at` is set to `now`.
/// The field set is replaced wholesale by `input.fields` (added/removed/relabeled
/// fields all take effect), preserving the id of any field whose input carried
/// one. Returns [`EntryError::NotFound`] if no such entry exists.
pub fn update_entry(
    model: &mut VaultModel,
    id: &str,
    input: EntryInput,
    now: String,
) -> Result<Entry, EntryError> {
    let entry = model
        .entries
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or(EntryError::NotFound)?;

    entry.service_ref = input.service_ref;
    entry.title = input.title;
    entry.fields = build_fields(input.fields);
    entry.updated_at = now;
    Ok(entry.clone())
}

/// Delete the entry with the given `id` (Req 8.6).
///
/// Deletion confirmation is handled at the UI layer; this performs the removal
/// unconditionally. Returns [`EntryError::NotFound`] if no such entry exists.
pub fn delete_entry(model: &mut VaultModel, id: &str) -> Result<(), EntryError> {
    let before = model.entries.len();
    model.entries.retain(|e| e.id != id);
    if model.entries.len() == before {
        Err(EntryError::NotFound)
    } else {
        Ok(())
    }
}

/// Whether `entry` is "empty" for the purposes of the non-blocking save warning
/// (Req 5.6, 8.6).
///
/// An entry counts as empty when it has no user-visible content: no non-blank
/// title and no field whose value contains a non-whitespace character. Field
/// labels alone do not count as content (a prefilled-but-unfilled recommended
/// field set should still warn).
pub fn entry_is_empty(entry: &Entry) -> bool {
    let title_blank = entry
        .title
        .as_deref()
        .map(|t| t.trim().is_empty())
        .unwrap_or(true);
    let all_values_blank = entry.fields.iter().all(|f| f.value.trim().is_empty());
    title_blank && all_values_blank
}

// ===========================================================================
// Task 6.2 — search index and paginated listing
// ===========================================================================
//
// ## Why the index excludes secrets (Req 9.4, Property 7)
//
// The list view must let users find entries fast (Req 9.1, 9.2) without ever
// putting a secret value where a substring search could surface it. The index
// is therefore built from **non-secret fields only**: a field marked
// `secret: true` contributes *neither its value nor its label* to the searchable
// document, and never to a summary. Combined with [`EntrySummary`] carrying no
// field values at all (only id, service ref, title, and a non-secret snippet),
// this upholds the invariant that "no secret field ever appears in the search
// index or in `list_entries` summaries" (Property 7).
//
// The index is rebuilt from the in-memory decrypted model on each call. The
// model is the single source of truth and already lives in memory while
// unlocked, so rebuilding avoids any risk of a stale index drifting from the
// data — and keeps these functions pure over a borrowed model, exactly like the
// CRUD functions above.

/// Default page size when a caller omits an explicit limit. Large enough that a
/// virtualized list has plenty to render, small enough to stay lightweight.
const DEFAULT_PAGE_LIMIT: usize = 50;

/// Maximum length (in characters) of the non-secret preview [`EntrySummary::snippet`].
const SNIPPET_MAX_CHARS: usize = 80;

/// A pagination request: a zero-based `offset` into the filtered result set and
/// a maximum number of items to return (`limit`).
///
/// Both fields default (offset `0`, limit [`DEFAULT_PAGE_LIMIT`]) so the
/// frontend can send a partial or absent page. Deserialized from the camelCase
/// wire shape `{ "offset": n, "limit": m }`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// Zero-based index of the first item to return from the filtered set.
    #[serde(default)]
    pub offset: usize,
    /// Maximum number of items to return.
    #[serde(default = "default_page_limit")]
    pub limit: usize,
}

fn default_page_limit() -> usize {
    DEFAULT_PAGE_LIMIT
}

impl Default for Page {
    fn default() -> Self {
        Page {
            offset: 0,
            limit: DEFAULT_PAGE_LIMIT,
        }
    }
}

/// A lightweight, **non-secret** summary of an [`Entry`] for the list view
/// (Req 9.3, Property 7).
///
/// Deliberately omits all field values: the list shows only enough to identify
/// an entry (its service, optional title, and a short non-secret snippet). Full
/// details — including secrets — are fetched on demand via `get_entry` when the
/// user opens an entry, minimizing decrypted secret material held by the UI.
///
/// Serializes to the camelCase shape the frontend `EntrySummary` expects
/// (`id`, `serviceRef`, optional `title`, optional `snippet`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    /// The entry's stable id.
    pub id: String,
    /// Which service this entry is for, so the UI can resolve the icon/name.
    pub service_ref: ServiceRef,
    /// Optional user label, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A short, non-secret preview drawn from the first non-secret field value.
    /// Never contains a secret value. Omitted when there is nothing to preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// A page of [`EntrySummary`] results plus the pagination metadata the frontend
/// needs to drive a virtualized/paged list.
///
/// `total` is the number of entries matching the filter **before** pagination,
/// so the UI can size its scrollbar / show counts; `entries` is the requested
/// slice (`offset`..`offset + limit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryList {
    /// The summaries for the requested page (post-filter, post-pagination).
    pub entries: Vec<EntrySummary>,
    /// Total number of entries matching the filter, before pagination.
    pub total: usize,
    /// The offset that was applied.
    pub offset: usize,
    /// The limit that was applied.
    pub limit: usize,
}

/// An in-memory search index over the vault's entries, built from **non-secret
/// content only** (Req 9.4, Property 7).
///
/// Each entry contributes one lowercased "document" composed of its service
/// reference (catalog id, or custom-service id + resolved name), its title, and
/// the labels and values of its **non-secret** fields. Secret fields contribute
/// nothing. Filtering is a case-insensitive AND-of-terms substring match against
/// these documents.
pub struct SearchIndex {
    docs: Vec<IndexedEntry>,
}

/// One indexed entry: the position of the entry in `model.entries` paired with
/// its lowercased, non-secret searchable text.
struct IndexedEntry {
    entry_index: usize,
    text: String,
}

impl SearchIndex {
    /// Build the index from the decrypted `model`, preserving entry order.
    pub fn build(model: &VaultModel) -> Self {
        let docs = model
            .entries
            .iter()
            .enumerate()
            .map(|(entry_index, entry)| IndexedEntry {
                entry_index,
                text: indexed_text(model, entry),
            })
            .collect();
        SearchIndex { docs }
    }

    /// Indices (into `model.entries`) of entries matching `query`, in original
    /// model order.
    ///
    /// An absent or blank query matches every entry. Otherwise the query is split
    /// into whitespace-separated terms and an entry matches only when **every**
    /// term is a case-insensitive substring of its indexed document (AND
    /// semantics), so adding terms narrows results.
    pub fn matching_indices(&self, query: Option<&str>) -> Vec<usize> {
        let terms: Vec<String> = query
            .unwrap_or("")
            .split_whitespace()
            .map(|t| t.to_lowercase())
            .collect();

        self.docs
            .iter()
            .filter(|doc| terms.iter().all(|term| doc.text.contains(term.as_str())))
            .map(|doc| doc.entry_index)
            .collect()
    }
}

/// Compose the lowercased, **non-secret** searchable document for `entry`.
///
/// Includes: the service reference (catalog id, or custom-service id plus its
/// resolved display name), the title, and the label + value of each non-secret
/// field. Secret fields are skipped entirely (Property 7).
fn indexed_text(model: &VaultModel, entry: &Entry) -> String {
    let mut parts: Vec<String> = Vec::new();

    match &entry.service_ref {
        ServiceRef::Catalog { id } => parts.push(id.clone()),
        ServiceRef::Custom { id } => {
            parts.push(id.clone());
            if let Some(service) = model.custom_services.iter().find(|s| &s.id == id) {
                parts.push(service.name.clone());
            }
        }
    }

    if let Some(title) = &entry.title {
        parts.push(title.clone());
    }

    for field in &entry.fields {
        if !field.secret {
            parts.push(field.label.clone());
            parts.push(field.value.clone());
        }
    }

    parts.join("\n").to_lowercase()
}

/// Build a short, non-secret preview for a summary: the first non-secret field
/// with a non-blank value, truncated to [`SNIPPET_MAX_CHARS`] characters. Secret
/// fields are never considered, so a snippet can never leak a secret value.
fn build_snippet(entry: &Entry) -> Option<String> {
    entry
        .fields
        .iter()
        .find(|f| !f.secret && !f.value.trim().is_empty())
        .map(|f| truncate_chars(f.value.trim(), SNIPPET_MAX_CHARS))
}

/// Truncate `s` to at most `max` characters (not bytes), appending an ellipsis
/// when content was dropped. Operates on `char`s so multi-byte text is never
/// split mid-codepoint.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// Build a non-secret [`EntrySummary`] from a stored [`Entry`] (Property 7).
fn summarize_entry(entry: &Entry) -> EntrySummary {
    EntrySummary {
        id: entry.id.clone(),
        service_ref: entry.service_ref.clone(),
        title: entry.title.clone(),
        snippet: build_snippet(entry),
    }
}

/// Filtered, paginated listing of entries as lightweight, non-secret summaries
/// (Req 9.1, 9.2, 9.3, 9.4).
///
/// Builds the non-secret [`SearchIndex`], filters by `query` (matching service
/// name/ref, field labels, and non-secret field values — never secret values),
/// then returns the `page` slice of [`EntrySummary`]s together with the total
/// match count for the UI. An absent `query` lists everything; an absent `page`
/// uses the default offset/limit.
pub fn list_entries(model: &VaultModel, query: Option<String>, page: Option<Page>) -> EntryList {
    let index = SearchIndex::build(model);
    let matched = index.matching_indices(query.as_deref());
    let total = matched.len();

    let page = page.unwrap_or_default();
    let entries = matched
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .map(|i| summarize_entry(&model.entries[i]))
        .collect();

    EntryList {
        entries,
        total,
        offset: page.offset,
        limit: page.limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::VaultModel;

    fn field_input(label: &str, value: &str, secret: bool) -> FieldInput {
        FieldInput {
            id: None,
            label: label.to_string(),
            field_type: FieldType::Text,
            value: value.to_string(),
            secret,
        }
    }

    fn sample_input() -> EntryInput {
        EntryInput {
            service_ref: ServiceRef::Catalog {
                id: "facebook".to_string(),
            },
            title: Some("Personal".to_string()),
            fields: vec![
                FieldInput {
                    id: None,
                    label: "Email".to_string(),
                    field_type: FieldType::Email,
                    value: "me@example.com".to_string(),
                    secret: false,
                },
                FieldInput {
                    id: None,
                    label: "Password".to_string(),
                    field_type: FieldType::Password,
                    value: "hunter2".to_string(),
                    secret: true,
                },
            ],
        }
    }

    #[test]
    fn create_assigns_uuid_and_timestamps() {
        let mut model = VaultModel::new();
        let entry = create_entry(&mut model, sample_input(), "2024-01-01T00:00:00Z".to_string());

        // The entry is persisted in the model.
        assert_eq!(model.entries.len(), 1);
        // A UUID v4 id is assigned (parseable as a UUID).
        assert!(Uuid::parse_str(&entry.id).is_ok(), "entry id must be a UUID");
        // created_at == updated_at at creation (Req 5.5).
        assert_eq!(entry.created_at, "2024-01-01T00:00:00Z");
        assert_eq!(entry.updated_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn create_assigns_unique_field_ids() {
        let mut model = VaultModel::new();
        let entry = create_entry(&mut model, sample_input(), "t".to_string());
        assert_eq!(entry.fields.len(), 2);
        for field in &entry.fields {
            assert!(Uuid::parse_str(&field.id).is_ok(), "field id must be a UUID");
        }
        assert_ne!(
            entry.fields[0].id, entry.fields[1].id,
            "each field gets a distinct id"
        );
        // The secret flag and custom labels round-trip into the stored field.
        assert_eq!(entry.fields[1].label, "Password");
        assert!(entry.fields[1].secret);
    }

    #[test]
    fn distinct_entries_get_distinct_ids() {
        let mut model = VaultModel::new();
        let a = create_entry(&mut model, sample_input(), "t".to_string());
        let b = create_entry(&mut model, sample_input(), "t".to_string());
        assert_ne!(a.id, b.id, "each created entry gets a unique id");
        assert_eq!(model.entries.len(), 2);
    }

    #[test]
    fn get_entry_finds_existing_and_misses_unknown() {
        let mut model = VaultModel::new();
        let created = create_entry(&mut model, sample_input(), "t".to_string());

        let found = get_entry(&model, &created.id).expect("entry should be found");
        assert_eq!(found.id, created.id);
        assert_eq!(found.title.as_deref(), Some("Personal"));

        assert!(get_entry(&model, "no-such-id").is_none());
    }

    #[test]
    fn update_preserves_created_at_and_id_and_bumps_updated_at() {
        let mut model = VaultModel::new();
        let created = create_entry(&mut model, sample_input(), "2024-01-01T00:00:00Z".to_string());

        let mut input = sample_input();
        input.title = Some("Renamed".to_string());
        let updated = update_entry(
            &mut model,
            &created.id,
            input,
            "2024-06-15T09:30:00Z".to_string(),
        )
        .expect("update should succeed");

        // Identity and creation time are stable; only updated_at moves (Req 8.5).
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.created_at, "2024-01-01T00:00:00Z");
        assert_eq!(updated.updated_at, "2024-06-15T09:30:00Z");
        assert_eq!(updated.title.as_deref(), Some("Renamed"));
        // The change is reflected in the stored model, not just the return value.
        assert_eq!(model.entries[0].title.as_deref(), Some("Renamed"));
        assert_eq!(model.entries[0].updated_at, "2024-06-15T09:30:00Z");
    }

    #[test]
    fn update_can_add_remove_and_relabel_fields() {
        let mut model = VaultModel::new();
        let created = create_entry(&mut model, sample_input(), "t".to_string());
        let keep_id = created.fields[0].id.clone();

        // Keep the first field (preserving its id, relabeled), drop the second,
        // and add a brand-new field.
        let input = EntryInput {
            service_ref: ServiceRef::Catalog {
                id: "facebook".to_string(),
            },
            title: Some("Personal".to_string()),
            fields: vec![
                FieldInput {
                    id: Some(keep_id.clone()),
                    label: "Primary email".to_string(),
                    field_type: FieldType::Email,
                    value: "me@example.com".to_string(),
                    secret: false,
                },
                field_input("Note", "some note", false),
            ],
        };

        let updated = update_entry(&mut model, &created.id, input, "t2".to_string())
            .expect("update should succeed");

        assert_eq!(updated.fields.len(), 2);
        // Preserved field keeps its id and takes the new label.
        assert_eq!(updated.fields[0].id, keep_id);
        assert_eq!(updated.fields[0].label, "Primary email");
        // Newly added field gets a fresh UUID, distinct from the preserved one.
        assert!(Uuid::parse_str(&updated.fields[1].id).is_ok());
        assert_ne!(updated.fields[1].id, keep_id);
    }

    #[test]
    fn update_unknown_entry_is_not_found() {
        let mut model = VaultModel::new();
        let err = update_entry(&mut model, "no-such-id", sample_input(), "t".to_string())
            .unwrap_err();
        assert_eq!(err, EntryError::NotFound);
    }

    #[test]
    fn delete_removes_only_the_target_entry() {
        let mut model = VaultModel::new();
        let a = create_entry(&mut model, sample_input(), "t".to_string());
        let b = create_entry(&mut model, sample_input(), "t".to_string());

        delete_entry(&mut model, &a.id).expect("delete should succeed");

        assert_eq!(model.entries.len(), 1);
        assert_eq!(model.entries[0].id, b.id, "the other entry is untouched");
        // The deleted entry is gone.
        assert!(get_entry(&model, &a.id).is_none());
    }

    #[test]
    fn delete_unknown_entry_is_not_found() {
        let mut model = VaultModel::new();
        let err = delete_entry(&mut model, "no-such-id").unwrap_err();
        assert_eq!(err, EntryError::NotFound);
    }

    #[test]
    fn empty_entry_warning_detects_blank_entries() {
        let mut model = VaultModel::new();

        // No title, all field values blank/whitespace → empty (warn).
        let blank = create_entry(
            &mut model,
            EntryInput {
                service_ref: ServiceRef::Catalog {
                    id: "facebook".to_string(),
                },
                title: None,
                fields: vec![field_input("Email", "   ", false), field_input("Note", "", false)],
            },
            "t".to_string(),
        );
        assert!(entry_is_empty(&blank), "blank values must be flagged empty");

        // A filled field value → not empty.
        let filled = create_entry(
            &mut model,
            EntryInput {
                service_ref: ServiceRef::Catalog {
                    id: "facebook".to_string(),
                },
                title: None,
                fields: vec![field_input("Email", "me@example.com", false)],
            },
            "t".to_string(),
        );
        assert!(!entry_is_empty(&filled));
    }

    #[test]
    fn entry_with_no_fields_is_empty() {
        let mut model = VaultModel::new();
        let entry = create_entry(
            &mut model,
            EntryInput {
                service_ref: ServiceRef::Catalog {
                    id: "x".to_string(),
                },
                title: None,
                fields: vec![],
            },
            "t".to_string(),
        );
        assert!(entry_is_empty(&entry));
    }

    // ---- Task 6.2: search index and paginated listing ----

    /// Push a fully-formed entry directly into the model (bypassing the input
    /// path) so tests can control ids deterministically for assertions.
    fn push_entry(
        model: &mut VaultModel,
        id: &str,
        service_ref: ServiceRef,
        title: Option<&str>,
        fields: Vec<Field>,
    ) {
        model.entries.push(Entry {
            id: id.to_string(),
            service_ref,
            title: title.map(|t| t.to_string()),
            fields,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        });
    }

    fn field(id: &str, label: &str, value: &str, secret: bool) -> Field {
        Field {
            id: id.to_string(),
            label: label.to_string(),
            field_type: if secret {
                FieldType::Password
            } else {
                FieldType::Text
            },
            value: value.to_string(),
            secret,
        }
    }

    /// A small model with a secret and a non-secret field on one entry, plus a
    /// custom-service entry, used across the search tests.
    fn search_model() -> VaultModel {
        let mut model = VaultModel::new();
        model.custom_services.push(crate::model::CustomService {
            id: "svc-home".to_string(),
            name: "My Home Server".to_string(),
            icon: crate::model::IconRef::Builtin {
                reference: "server.svg".to_string(),
            },
        });
        push_entry(
            &mut model,
            "e-fb",
            ServiceRef::Catalog {
                id: "facebook".to_string(),
            },
            Some("Personal"),
            vec![
                field("f1", "Email", "alice@example.com", false),
                field("f2", "Password", "S3cret-Passw0rd", true),
            ],
        );
        push_entry(
            &mut model,
            "e-home",
            ServiceRef::Custom {
                id: "svc-home".to_string(),
            },
            None,
            vec![
                field("f3", "Username", "homeadmin", false),
                field("f4", "Recovery code", "TOPSECRETCODE", true),
            ],
        );
        model
    }

    /// Property 7 / Req 9.4: a secret field's **value** is never indexed, so a
    /// search for it returns nothing — even though the value exists on an entry.
    ///
    /// **Validates: Requirements 9.4**
    #[test]
    fn secret_values_are_never_indexed_or_matched() {
        let model = search_model();

        // The exact secret values exist on entries...
        assert_eq!(model.entries[0].fields[1].value, "S3cret-Passw0rd");
        assert_eq!(model.entries[1].fields[1].value, "TOPSECRETCODE");

        // ...but searching for them matches nothing.
        for needle in ["S3cret-Passw0rd", "Passw0rd", "TOPSECRETCODE", "secretcode"] {
            let result = list_entries(&model, Some(needle.to_string()), None);
            assert_eq!(
                result.total, 0,
                "secret value '{needle}' must not be searchable"
            );
            assert!(result.entries.is_empty());
        }

        // And the raw index documents contain neither secret value.
        let index = SearchIndex::build(&model);
        for doc in &index.docs {
            assert!(
                !doc.text.contains("s3cret-passw0rd"),
                "index must not contain the password value"
            );
            assert!(
                !doc.text.contains("topsecretcode"),
                "index must not contain the recovery code value"
            );
        }
    }

    /// Property 7: labels of secret fields are also excluded from the index, so
    /// the whole secret field never appears (stronger than just hiding values).
    #[test]
    fn secret_field_labels_are_not_indexed() {
        let mut model = VaultModel::new();
        push_entry(
            &mut model,
            "e1",
            ServiceRef::Catalog {
                id: "example".to_string(),
            },
            None,
            vec![field("f1", "Backup MFA Seed", "abc", true)],
        );
        // The secret field's label must not make the entry findable.
        let result = list_entries(&model, Some("Backup MFA Seed".to_string()), None);
        assert_eq!(result.total, 0, "secret field labels must not be indexed");
    }

    /// Summaries carry no field values at all, and the non-secret snippet never
    /// equals a secret value (Property 7, Req 9.3).
    #[test]
    fn summaries_exclude_secret_values_and_snippet_is_non_secret() {
        let model = search_model();
        let result = list_entries(&model, None, None);

        let fb = result
            .entries
            .iter()
            .find(|s| s.id == "e-fb")
            .expect("facebook summary present");
        // Snippet comes from the first non-secret field value, never the password.
        assert_eq!(fb.snippet.as_deref(), Some("alice@example.com"));
        assert_ne!(fb.snippet.as_deref(), Some("S3cret-Passw0rd"));
        assert_eq!(fb.title.as_deref(), Some("Personal"));

        // The home entry's first non-secret field is the username.
        let home = result
            .entries
            .iter()
            .find(|s| s.id == "e-home")
            .expect("home summary present");
        assert_eq!(home.snippet.as_deref(), Some("homeadmin"));
    }

    /// An entry whose only fields are secret yields no snippet (nothing
    /// non-secret to preview).
    #[test]
    fn entry_with_only_secret_fields_has_no_snippet() {
        let mut model = VaultModel::new();
        push_entry(
            &mut model,
            "e1",
            ServiceRef::Catalog {
                id: "example".to_string(),
            },
            None,
            vec![field("f1", "Password", "hunter2", true)],
        );
        let result = list_entries(&model, None, None);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].snippet, None);
    }

    /// Filtering matches by service name/ref, title, non-secret label, and
    /// non-secret value (Req 9.1), case-insensitively (Req 9.2).
    #[test]
    fn filtering_matches_service_title_label_and_nonsecret_value() {
        let model = search_model();

        // By catalog service ref id.
        let by_service = list_entries(&model, Some("facebook".to_string()), None);
        assert_eq!(by_service.total, 1);
        assert_eq!(by_service.entries[0].id, "e-fb");

        // By resolved custom-service name (case-insensitive).
        let by_custom = list_entries(&model, Some("home server".to_string()), None);
        assert_eq!(by_custom.total, 1);
        assert_eq!(by_custom.entries[0].id, "e-home");

        // By title.
        let by_title = list_entries(&model, Some("PERSONAL".to_string()), None);
        assert_eq!(by_title.total, 1);
        assert_eq!(by_title.entries[0].id, "e-fb");

        // By non-secret field label.
        let by_label = list_entries(&model, Some("username".to_string()), None);
        assert_eq!(by_label.total, 1);
        assert_eq!(by_label.entries[0].id, "e-home");

        // By non-secret field value.
        let by_value = list_entries(&model, Some("alice@example.com".to_string()), None);
        assert_eq!(by_value.total, 1);
        assert_eq!(by_value.entries[0].id, "e-fb");
    }

    /// A blank or absent query lists everything; multi-term queries AND together.
    #[test]
    fn empty_query_lists_all_and_terms_are_anded() {
        let model = search_model();

        assert_eq!(list_entries(&model, None, None).total, 2);
        assert_eq!(list_entries(&model, Some("   ".to_string()), None).total, 2);

        // Both terms present on the facebook entry (service + title) → match.
        let both = list_entries(&model, Some("facebook personal".to_string()), None);
        assert_eq!(both.total, 1);
        assert_eq!(both.entries[0].id, "e-fb");

        // One term that matches no single entry's document → no match (AND).
        let none = list_entries(&model, Some("facebook homeadmin".to_string()), None);
        assert_eq!(none.total, 0);
    }

    /// A query matching nothing returns an empty page with total 0.
    #[test]
    fn nonmatching_query_returns_empty() {
        let model = search_model();
        let result = list_entries(&model, Some("nonexistent-zzz".to_string()), None);
        assert_eq!(result.total, 0);
        assert!(result.entries.is_empty());
    }

    /// Pagination returns the correct slice and reports total/offset/limit; the
    /// total reflects all matches, not just the returned page (Req 9.3).
    #[test]
    fn pagination_returns_correct_slice_and_total() {
        let mut model = VaultModel::new();
        for i in 0..10 {
            push_entry(
                &mut model,
                &format!("e{i:02}"),
                ServiceRef::Catalog {
                    id: "svc".to_string(),
                },
                Some(&format!("Entry {i}")),
                vec![field("f", "Note", &format!("value {i}"), false)],
            );
        }

        // First page of 3.
        let p0 = list_entries(
            &model,
            None,
            Some(Page {
                offset: 0,
                limit: 3,
            }),
        );
        assert_eq!(p0.total, 10, "total counts all matches, not the page size");
        assert_eq!(p0.offset, 0);
        assert_eq!(p0.limit, 3);
        let ids: Vec<_> = p0.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e00", "e01", "e02"]);

        // Middle page.
        let p1 = list_entries(
            &model,
            None,
            Some(Page {
                offset: 3,
                limit: 3,
            }),
        );
        let ids: Vec<_> = p1.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e03", "e04", "e05"]);

        // Last partial page (offset near the end returns only what remains).
        let p3 = list_entries(
            &model,
            None,
            Some(Page {
                offset: 9,
                limit: 3,
            }),
        );
        assert_eq!(p3.entries.len(), 1);
        assert_eq!(p3.entries[0].id, "e09");
        assert_eq!(p3.total, 10);

        // Offset past the end yields an empty page but still the real total.
        let past = list_entries(
            &model,
            None,
            Some(Page {
                offset: 50,
                limit: 3,
            }),
        );
        assert!(past.entries.is_empty());
        assert_eq!(past.total, 10);
    }

    /// Pagination is applied to the filtered set, not the whole vault.
    #[test]
    fn pagination_applies_after_filtering() {
        let mut model = VaultModel::new();
        // 5 matching ("apple") interleaved with 5 non-matching ("banana").
        for i in 0..5 {
            push_entry(
                &mut model,
                &format!("apple{i}"),
                ServiceRef::Catalog {
                    id: "apple".to_string(),
                },
                None,
                vec![field("f", "Note", "x", false)],
            );
            push_entry(
                &mut model,
                &format!("banana{i}"),
                ServiceRef::Catalog {
                    id: "banana".to_string(),
                },
                None,
                vec![field("f", "Note", "x", false)],
            );
        }

        let page = list_entries(
            &model,
            Some("apple".to_string()),
            Some(Page {
                offset: 2,
                limit: 2,
            }),
        );
        assert_eq!(page.total, 5, "only the 5 apple entries match");
        let ids: Vec<_> = page.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["apple2", "apple3"]);
    }

    /// The default page (no `Page` supplied) uses offset 0 and the default limit.
    #[test]
    fn default_page_uses_default_limit() {
        let mut model = VaultModel::new();
        for i in 0..(DEFAULT_PAGE_LIMIT + 10) {
            push_entry(
                &mut model,
                &format!("e{i}"),
                ServiceRef::Catalog {
                    id: "svc".to_string(),
                },
                None,
                vec![],
            );
        }
        let result = list_entries(&model, None, None);
        assert_eq!(result.total, DEFAULT_PAGE_LIMIT + 10);
        assert_eq!(result.offset, 0);
        assert_eq!(result.limit, DEFAULT_PAGE_LIMIT);
        assert_eq!(result.entries.len(), DEFAULT_PAGE_LIMIT);
    }

    /// Summary order follows model order so the list is stable/predictable.
    #[test]
    fn summaries_preserve_model_order() {
        let model = search_model();
        let result = list_entries(&model, None, None);
        let ids: Vec<_> = result.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e-fb", "e-home"]);
    }

    /// A long snippet value is truncated to the snippet character budget.
    #[test]
    fn snippet_is_truncated() {
        let mut model = VaultModel::new();
        let long = "a".repeat(SNIPPET_MAX_CHARS + 50);
        push_entry(
            &mut model,
            "e1",
            ServiceRef::Catalog {
                id: "svc".to_string(),
            },
            None,
            vec![field("f", "Note", &long, false)],
        );
        let result = list_entries(&model, None, None);
        let snippet = result.entries[0].snippet.as_deref().unwrap();
        // Truncated to SNIPPET_MAX_CHARS chars plus a single ellipsis marker.
        assert_eq!(snippet.chars().count(), SNIPPET_MAX_CHARS + 1);
        assert!(snippet.ends_with('…'));
    }
}
