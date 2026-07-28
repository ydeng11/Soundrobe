//! Deterministic intent and scope routing for the AI assistant harness.
//!
//! Routes unambiguous user requests to concrete tools before the LLM is
//! consulted. Supported patterns:
//!   - `set <field> to <literal>` / `set <field> <literal>`
//!   - `change <field> to <literal>`
//!   - `remove/clear <field>`
//!   - `set <field> where missing` / `fix the missing <field>` (requires follow-up for value)
//!   - `set them <literal>` — refers to the previous result-set referent.
//!
//! All quoted values are preserved exactly (outer quotes removed, commas kept).
//! Ambiguous requests fall through to `NotRouted` so the LLM decides.

use crate::commands::assistant::AssistantSendInput;
use crate::state::assistant_task::{ResolvedIntent, ScopePredicate, SessionState};
use serde_json::Value;

/// The result of deterministic parsing: either a resolved intent with scope
/// and action details, or a `NotRouted` that defers to the LLM.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedCommand {
    pub intent: IntentKind,
    pub field: Option<String>,
    pub value: Option<String>,
    pub only_if_missing: bool,
    pub scope_hint: ScopeHint,
    /// Whether this was a referent-referencing command ("set them X").
    pub uses_referent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum IntentKind {
    SetField,
    RemoveField,
    ClearField,
    SetMissing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScopeHint {
    Selected,
    ActiveAlbum,
    Library,
    FromPredicate(ScopePredicate),
    Referent,
}

// ── Parsing utilities ─────────────────────────────────────────────

/// Strip balanced outer quotes from a string value.
/// `"Pop, Cantopop"` → `Pop, Cantopop` (keeping the comma)
/// `'Pop, Cantopop'` → `Pop, Cantopop`
/// `Pop` → `Pop` (no quotes)
pub fn strip_outer_quotes(value: &str) -> String {
    let trimmed = value.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        let inner = &trimmed[1..trimmed.len() - 1];
        inner.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Known metadata fields that can be set/removed.
const KNOWN_FIELDS: &[&str] = &[
    "title", "artist", "album", "genre", "year", "composer",
    "comment", "description", "lyrics", "albumArtist", "albumArtists",
    "artists", "trackNumber", "trackTotal", "discNumber", "discTotal",
    "compilation",
];

// ── Token-level intent parsing ─────────────────────────────────────

/// Represents parsed tokens from a user request.
#[derive(Debug, Clone, PartialEq)]
struct ParsedRequest {
    /// The action verb (set, change, remove, clear, fix).
    action: String,
    /// The field, if identifiable.
    field: Option<String>,
    /// The value, if provided.
    value: Option<String>,
    /// Whether "missing" was mentioned.
    missing_qualifier: bool,
    /// Whether "them" was used as the target.
    them_referent: bool,
}

/// Parse a user request into tokens for further processing.
/// This is a conservative, pattern-based parser — it does not use LLM.
/// Preserves original case for values while matching case-insensitively.
fn parse_request_tokens(message: &str) -> ParsedRequest {
    let trimmed = message.trim();
    let lower = trimmed.to_lowercase();

    // Detect "them" referent usage (case-insensitive)
    let them_referent = lower.starts_with("set them ")
        || lower.starts_with("change them ")
        || lower == "set them";

    // Detect "missing" qualifier (case-insensitive)
    let missing_qualifier = lower.contains(" where missing")
        || lower.contains("missing ")
        || lower.ends_with("missing");

    // Detect action verb
    let action: &str = if lower.starts_with("set ") || lower.starts_with("set the ") {
        "set"
    } else if lower.starts_with("change ") || lower.starts_with("change the ") {
        "set"
    } else if lower.starts_with("remove ") || lower.starts_with("remove the ") {
        "remove"
    } else if lower.starts_with("clear ") || lower.starts_with("clear the ") {
        "remove"
    } else if lower.starts_with("fix ") || lower.starts_with("fix the ") {
        "fix"
    } else {
        return ParsedRequest {
            action: String::new(),
            field: None,
            value: None,
            missing_qualifier: false,
            them_referent: false,
        };
    };
    let action = action.to_string();

    // Extract the body after the action verb from the ORIGINAL (preserving case)
    let body = if them_referent && lower.starts_with("set them ") {
        // "set them <value>" — body after "set them " in original
        let prefix_end = "set them ".len();
        let original_prefix = &trimmed[..prefix_end.min(trimmed.len())];
        if original_prefix.len() >= prefix_end {
            trimmed[prefix_end..].to_string()
        } else {
            String::new()
        }
    } else if lower.starts_with("set the ") {
        trimmed.strip_prefix("set the ").or_else(|| trimmed.strip_prefix("Set the ")).unwrap_or("").to_string()
    } else if lower.starts_with("set ") {
        // Use original case: strip the word "set" (case-insensitive)
        let prefix_len = if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("set ") {
            4
        } else if trimmed.len() >= 8 && trimmed[..8].eq_ignore_ascii_case("set the ") {
            8
        } else {
            4
        };
        trimmed[prefix_len.min(trimmed.len())..].to_string()
    } else if lower.starts_with("change the ") {
        let prefix_len = if trimmed.len() >= 10 && trimmed[..10].eq_ignore_ascii_case("change the ") { 10 } else { 7 };
        trimmed[prefix_len.min(trimmed.len())..].to_string()
    } else if lower.starts_with("change ") {
        trimmed.strip_prefix("change ").or_else(|| trimmed.strip_prefix("Change ")).unwrap_or("").to_string()
    } else if lower.starts_with("remove the ") {
        let prefix_len = if trimmed.len() >= 10 && trimmed[..10].eq_ignore_ascii_case("remove the ") { 10 } else { 7 };
        trimmed[prefix_len.min(trimmed.len())..].to_string()
    } else if lower.starts_with("remove ") {
        trimmed.strip_prefix("remove ").or_else(|| trimmed.strip_prefix("Remove ")).unwrap_or("").to_string()
    } else if lower.starts_with("clear the ") {
        let prefix_len = if trimmed.len() >= 9 && trimmed[..9].eq_ignore_ascii_case("clear the ") { 9 } else { 6 };
        trimmed[prefix_len.min(trimmed.len())..].to_string()
    } else if lower.starts_with("clear ") {
        trimmed.strip_prefix("clear ").or_else(|| trimmed.strip_prefix("Clear ")).unwrap_or("").to_string()
    } else if lower.starts_with("fix the ") {
        let prefix_len = if trimmed.len() >= 8 && trimmed[..8].eq_ignore_ascii_case("fix the ") { 8 } else { 4 };
        trimmed[prefix_len.min(trimmed.len())..].to_string()
    } else if lower.starts_with("fix ") {
        trimmed.strip_prefix("fix ").or_else(|| trimmed.strip_prefix("Fix ")).unwrap_or("").to_string()
    } else {
        return ParsedRequest {
            action: String::new(),
            field: None,
            value: None,
            missing_qualifier: false,
            them_referent: false,
        };
    };

    if them_referent {
        // "set them <value>" — no field, uses referent
        let value = if body.trim().is_empty() {
            None
        } else {
            Some(body)
        };
        return ParsedRequest {
            action,
            field: None,
            value,
            missing_qualifier: false,
            them_referent: true,
        };
    }

    // Now parse body to find field and value
    // Try patterns:
    // 1. "<field> to <value>" — "genre to Pop, Cantopop"
    // 2. "<field> <value>" — "genre Pop, Cantopop"
    // 3. "the missing <field>" — "the missing genre" (no value)

    let body = body.trim();

    // Pattern: "the missing <field>[ to <value>]" — fix the missing genre [to X]
    if let Some(after_the_missing) = body.strip_prefix("the missing ") {
        let (field, rest) = extract_field_and_rest(after_the_missing);
        let value = rest.and_then(|r| r.strip_prefix("to ").map(str::to_string));
        return ParsedRequest {
            action,
            field,
            value,
            missing_qualifier: true,
            them_referent: false,
        };
    }

    // Pattern: "missing <field>[ to <value>]" — when "set the" or "fix the" consumed "the"
    // Use extract_field_and_rest to get the field name and any remaining value.
    if let Some(after_missing) = body.strip_prefix("missing ") {
        let (field, rest) = extract_field_and_rest(after_missing);
        let value = rest.and_then(|r| r.strip_prefix("to ").map(str::to_string));
        return ParsedRequest {
            action,
            field,
            value,
            missing_qualifier: true,
            them_referent: false,
        };
    }

    // Pattern: "<field> missing" / "missing <field>" without action verb
    if body == "missing" {
        return ParsedRequest {
            action: action.to_string(),
            field: None,
            value: None,
            missing_qualifier: true,
            them_referent: false,
        };
    }

    // Find the field by checking body start matches known fields
    let (field, rest) = extract_field_and_rest(body);

    let field = field.or_else(|| {
        // Maybe it's just "the <field>" — common in messages like "set the genre"
        if let Some(rest) = body.strip_prefix("the ") {
            identify_field(rest)
        } else {
            None
        }
    });

    if let Some(field) = &field {
        // Now find the value
        if let Some(rest) = rest {
            let trimmed_rest = rest.trim();
            // If the rest is just "where missing" or "missing", no value provided
            if trimmed_rest == "where missing" || trimmed_rest == "missing" {
                return ParsedRequest {
                    action,
                    field: Some(field.clone()),
                    value: None,
                    missing_qualifier: true,
                    them_referent: false,
                };
            }
            // Try "to <value>"
            let value = if let Some(val) = trimmed_rest.strip_prefix("to ") {
                Some(val.to_string())
            } else {
                // Otherwise the whole rest is the value
                Some(trimmed_rest.to_string())
            };
            return ParsedRequest {
                action,
                field: Some(field.clone()),
                value,
                missing_qualifier,
                them_referent: false,
            };
        }
    }

    ParsedRequest {
        action,
        field,
        value: None,
        missing_qualifier,
        them_referent: false,
    }
}

/// Try to identify a known field at the start of the body text.
/// Returns (field_name, rest_of_body) or (None, None) if no known field is found.
fn extract_field_and_rest(body: &str) -> (Option<String>, Option<String>) {
    // Multi-word fields first (check before single-word fields that might match prefixes)
    let multi_fields = &[
        ("album artist", "albumArtist"),
        ("album artists", "albumArtists"),
        ("track number", "trackNumber"),
        ("track total", "trackTotal"),
        ("disc number", "discNumber"),
        ("disc total", "discTotal"),
    ];
    for (field, canonical) in multi_fields {
        if let Some(raw_rest) = body.strip_prefix(field) {
            if raw_rest.is_empty() || raw_rest.starts_with(' ') || raw_rest.starts_with(" to ") {
                let val = raw_rest.trim().to_string();
                let rest = if val.is_empty() { None } else { Some(val) };
                return (Some(canonical.to_string()), rest);
            }
        }
    }

    for known in KNOWN_FIELDS {
        let lower_known: String = known.chars().flat_map(|c| c.to_lowercase()).collect();
        if let Some(raw_rest) = body.strip_prefix(&lower_known) {
            // Check the UNTRIMMED rest: must be empty, start with space, or start with " to ".
            if raw_rest.is_empty() || raw_rest.starts_with(' ') || raw_rest.starts_with(" to ") {
                let val = raw_rest.trim().to_string();
                let rest = if val.is_empty() { None } else { Some(val) };
                return (Some(known.to_string()), rest);
            }
        }
        // Also check with "the" prefix
        let the_prefix = format!("the {lower_known}");
        if let Some(raw_rest) = body.strip_prefix(&the_prefix) {
            if raw_rest.is_empty() || raw_rest.starts_with(' ') || raw_rest.starts_with(" to ") {
                let val = raw_rest.trim().to_string();
                let rest = if val.is_empty() { None } else { Some(val) };
                return (Some(known.to_string()), rest);
            }
        }
    }

    (None, None)
}

/// Identify a field name from a string that might contain qualifiers.
fn identify_field(text: &str) -> Option<String> {
    let trimmed = text.trim().to_lowercase();
    for known in KNOWN_FIELDS {
        let lower_known: String = known.chars().flat_map(|c| c.to_lowercase()).collect();
        if trimmed == lower_known
            || trimmed.starts_with(&lower_known) && trimmed.len() == lower_known.len()
        {
            return Some(known.to_string());
        }
    }
    // Multi-word
    let multi = &[
        ("albumartist", "albumArtist"),
        ("albumartists", "albumArtists"),
        ("tracknumber", "trackNumber"),
        ("tracktotal", "trackTotal"),
        ("discnumber", "discNumber"),
        ("disctotal", "discTotal"),
        ("album artist", "albumArtist"),
        ("album artists", "albumArtists"),
        ("track number", "trackNumber"),
        ("track total", "trackTotal"),
        ("disc number", "discNumber"),
        ("disc total", "discTotal"),
    ];
    for (variant, canonical) in multi {
        if trimmed == *variant {
            return Some(canonical.to_string());
        }
    }
    None
}

// ── Main routing entry point ──────────────────────────────────────

/// Route a user message deterministically.
///
/// Returns `Some(RoutedCommand)` if the message matches a known unambiguous
/// pattern, or `None` if it should be handled by the LLM.
///
/// When a referent is provided (from a previous session), `set them X`
/// commands can be resolved.
pub fn route_message(
    message: &str,
    referent: Option<&SessionState>,
) -> Option<RoutedCommand> {
    let tokens = parse_request_tokens(message);

    if tokens.action.is_empty() {
        return None;
    }

    // Handle "set them <value>" referent commands
    if tokens.them_referent {
        return route_them_command(&tokens, referent);
    }

    match tokens.action.as_str() {
        "set" | "change" => {
            let field = tokens.field.clone()?;
            let value = tokens.value.as_ref().map(|v| strip_outer_quotes(v));

            let scope_hint = if tokens.missing_qualifier {
                ScopeHint::FromPredicate(ScopePredicate::LibraryAndMissing {
                    field: field.clone(),
                })
            } else {
                ScopeHint::Library
            };

            Some(RoutedCommand {
                intent: if tokens.missing_qualifier && value.is_some() {
                    IntentKind::SetField
                } else if tokens.missing_qualifier {
                    IntentKind::SetMissing
                } else {
                    IntentKind::SetField
                },
                field: Some(field),
                value,
                only_if_missing: tokens.missing_qualifier,
                scope_hint,
                uses_referent: false,
            })
        }
        "remove" | "clear" => {
            let field = tokens.field.clone()?;

            Some(RoutedCommand {
                intent: IntentKind::RemoveField,
                field: Some(field),
                value: None,
                only_if_missing: false,
                scope_hint: ScopeHint::Library,
                uses_referent: false,
            })
        }
        "fix" => {
            // "fix the missing genre" — needs value, only identifies scope
            let field = tokens.field.clone();

            if tokens.missing_qualifier {
                if let Some(ref f) = field {
                    return Some(RoutedCommand {
                        intent: IntentKind::SetMissing,
                        field: Some(f.clone()),
                        value: None, // needs follow-up for value
                        only_if_missing: true,
                        scope_hint: ScopeHint::FromPredicate(ScopePredicate::LibraryAndMissing {
                            field: f.clone(),
                        }),
                        uses_referent: false,
                    });
                }
            }

            // Ambiguous "fix" — let LLM handle
            None
        }
        _ => None,
    }
}

/// Route a `set them <value>` command using the stored referent.
fn route_them_command(
    tokens: &ParsedRequest,
    referent: Option<&SessionState>,
) -> Option<RoutedCommand> {
    let referent = referent?;

    // Referent must have a non-zero count
    if referent.referent_count == 0 {
        return None;
    }

    let field = referent.referent_field.clone();
    let value = tokens.value.as_ref().map(|v| strip_outer_quotes(v));

    Some(RoutedCommand {
        intent: IntentKind::SetField,
        field,
        value,
        only_if_missing: true, // referent-based changes are typically only-if-missing
        scope_hint: ScopeHint::Referent,
        uses_referent: true,
    })
}

/// Determine the scope from a routed command and the current input.
pub fn resolve_scope(
    command: &RoutedCommand,
    input: &AssistantSendInput,
    referent: Option<&SessionState>,
) -> Vec<String> {
    match &command.scope_hint {
        ScopeHint::Selected => input.selected_track_paths.clone(),
        ScopeHint::ActiveAlbum => {
            input
                .tracks
                .iter()
                .filter_map(|t| t.get("path").and_then(Value::as_str))
                .filter(|p| input.active_album_path.as_deref().is_some_and(|a| p.starts_with(a)))
                .map(str::to_string)
                .collect()
        }
        ScopeHint::Library => {
            input
                .tracks
                .iter()
                .filter_map(|t| t.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        }
        ScopeHint::FromPredicate(predicate) => {
            let (paths, _count) = crate::state::assistant_task::evaluate_predicate(
                predicate,
                &input.tracks,
                input.active_album_path.as_deref(),
                &input.selected_track_paths,
            );
            paths
        }
        ScopeHint::Referent => {
            // For referent, we need to re-derive the paths from the session referent
            if let Some(ref_state) = referent {
                if let Some(query) = &ref_state.referent_query {
                    if query.contains("missing") {
                        if let Some(field) = &ref_state.referent_field {
                            let predicate = ScopePredicate::LibraryAndMissing {
                                field: field.clone(),
                            };
                            let (paths, _count) = crate::state::assistant_task::evaluate_predicate(
                                &predicate,
                                &input.tracks,
                                input.active_album_path.as_deref(),
                                &input.selected_track_paths,
                            );
                            return paths;
                        }
                    }
                }
            }
            Vec::new()
        }
    }
}

/// Convert a routed command into a `ResolvedIntent` for persistence.
pub fn resolved_intent_from_command(command: &RoutedCommand) -> ResolvedIntent {
    match command.intent {
        IntentKind::SetField => ResolvedIntent::SetField {
            field: command.field.clone().unwrap_or_default(),
            value: command.value.clone().unwrap_or_default(),
            only_if_missing: command.only_if_missing,
        },
        IntentKind::RemoveField | IntentKind::ClearField => ResolvedIntent::RemoveField {
            field: command.field.clone().unwrap_or_default(),
        },
        IntentKind::SetMissing => {
            if let Some(value) = &command.value {
                ResolvedIntent::SetField {
                    field: command.field.clone().unwrap_or_default(),
                    value: value.clone(),
                    only_if_missing: true,
                }
            } else {
                ResolvedIntent::NotRouted
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_input() -> AssistantSendInput {
        AssistantSendInput::default()
    }

    // ── quote stripping ────────────────────────────────────────────

    #[test]
    fn strips_double_quotes_preserving_commas() {
        assert_eq!(strip_outer_quotes(r#""Pop, Cantopop""#), "Pop, Cantopop");
    }

    #[test]
    fn strips_single_quotes_preserving_commas() {
        assert_eq!(strip_outer_quotes("'Pop, Cantopop'"), "Pop, Cantopop");
    }

    #[test]
    fn no_quotes_returns_trimmed() {
        assert_eq!(strip_outer_quotes("  Pop  "), "Pop");
    }

    #[test]
    fn unclosed_quote_returns_trimmed() {
        // Without balanced quotes, return trimmed verbatim
        assert_eq!(strip_outer_quotes("\"Pop, Cantopop"), "\"Pop, Cantopop");
    }

    // ── parse_request_tokens ───────────────────────────────────────

    #[test]
    fn parse_set_field_to_value() {
        let parsed = parse_request_tokens("set genre to Pop");
        assert_eq!(parsed.action, "set");
        assert_eq!(parsed.field.as_deref(), Some("genre"));
        assert_eq!(parsed.value.as_deref(), Some("Pop"));
        assert!(!parsed.missing_qualifier);
    }

    #[test]
    fn parse_set_field_value_without_to() {
        let parsed = parse_request_tokens("set genre Pop");
        assert_eq!(parsed.action, "set");
        assert_eq!(parsed.field.as_deref(), Some("genre"));
        assert_eq!(parsed.value.as_deref(), Some("Pop"));
    }

    #[test]
    fn parse_set_field_quoted_comma_value() {
        let parsed = parse_request_tokens(r#"set genre to "Pop, Cantopop""#);
        assert_eq!(parsed.action, "set");
        assert_eq!(parsed.field.as_deref(), Some("genre"));
        // The quoted value is preserved with quotes — stripping happens at route_message level
        assert_eq!(parsed.value.as_deref(), Some(r#""Pop, Cantopop""#));
    }

    #[test]
    fn parse_set_missing_field() {
        let parsed = parse_request_tokens("set genre where missing");
        assert_eq!(parsed.action, "set");
        assert_eq!(parsed.field.as_deref(), Some("genre"));
        assert!(parsed.missing_qualifier);
    }

    #[test]
    fn parse_change_field() {
        let parsed = parse_request_tokens("change title to New Title");
        assert_eq!(parsed.action, "set");
        assert_eq!(parsed.field.as_deref(), Some("title"));
        assert_eq!(parsed.value.as_deref(), Some("New Title"));
    }

    #[test]
    fn parse_remove_field() {
        let parsed = parse_request_tokens("remove genre");
        assert_eq!(parsed.action, "remove");
        assert_eq!(parsed.field.as_deref(), Some("genre"));
    }

    #[test]
    fn parse_clear_field() {
        let parsed = parse_request_tokens("clear comment");
        assert_eq!(parsed.action, "remove");
        assert_eq!(parsed.field.as_deref(), Some("comment"));
    }

    #[test]
    fn parse_fix_the_missing_field() {
        let parsed = parse_request_tokens("fix the missing genre");
        assert_eq!(parsed.action, "fix");
        assert_eq!(parsed.field.as_deref(), Some("genre"));
        assert!(parsed.missing_qualifier);
    }

    #[test]
    fn parse_set_them_referent() {
        let parsed = parse_request_tokens(r#"set them "Pop, Cantopop""#);
        assert_eq!(parsed.action, "set");
        assert!(parsed.them_referent);
        assert_eq!(parsed.value.as_deref(), Some(r#""Pop, Cantopop""#));
    }

    #[test]
    fn parse_unrecognized_returns_empty_action() {
        let parsed = parse_request_tokens("what genres are missing?");
        assert_eq!(parsed.action, "");
    }

    #[test]
    fn parse_multiple_words_field_names() {
        let parsed = parse_request_tokens("set album artist to Artist Name");
        assert_eq!(parsed.field.as_deref(), Some("albumArtist"));
    }

    // ── route_message ──────────────────────────────────────────────

    #[test]
    fn routes_set_field_explicit() {
        let cmd = route_message("set genre to Rock", None).unwrap();
        assert_eq!(cmd.intent, IntentKind::SetField);
        assert_eq!(cmd.field.as_deref(), Some("genre"));
        assert_eq!(cmd.value.as_deref(), Some("Rock"));
        assert!(!cmd.only_if_missing);
    }

    #[test]
    fn routes_set_missing_field() {
        let cmd = route_message("set genre where missing", None).unwrap();
        assert_eq!(cmd.intent, IntentKind::SetMissing);
        assert_eq!(cmd.field.as_deref(), Some("genre"));
        assert!(cmd.only_if_missing);
    }

    #[test]
    fn routes_set_missing_with_quoted_value() {
        let cmd = route_message(r#"set the missing genre to "Pop, Cantopop""#, None).unwrap();
        assert_eq!(cmd.intent, IntentKind::SetField);
        assert_eq!(cmd.field.as_deref(), Some("genre"));
        assert_eq!(cmd.value.as_deref(), Some("Pop, Cantopop"));
        assert!(cmd.only_if_missing);
    }

    #[test]
    fn routes_remove_field() {
        let cmd = route_message("remove genre", None).unwrap();
        assert_eq!(cmd.intent, IntentKind::RemoveField);
        assert_eq!(cmd.field.as_deref(), Some("genre"));
    }

    #[test]
    fn routes_them_with_referent() {
        let referent = SessionState {
            session_id: "s".to_string(),
            intent: Some("set_field".to_string()),
            scope_predicate: None,
            protocol: "native".to_string(),
            referent_count: 102,
            referent_query: Some("missing genre".to_string()),
            referent_field: Some("genre".to_string()),
            referent_value: None,
            pending_batch_ids: vec![],
            mutation_required: false,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        };
        let cmd = route_message(r#"set them "Pop, Cantopop""#, Some(&referent)).unwrap();
        assert_eq!(cmd.intent, IntentKind::SetField);
        assert_eq!(cmd.field.as_deref(), Some("genre"));
        assert_eq!(cmd.value.as_deref(), Some("Pop, Cantopop"));
        assert!(cmd.uses_referent);
    }

    #[test]
    fn routes_them_without_referent_falls_through() {
        let cmd = route_message(r#"set them "Pop, Cantopop""#, None);
        assert!(cmd.is_none(), "without referent, them command should not route");
    }

    #[test]
    fn ambiguous_fix_does_not_route() {
        let cmd = route_message("fix the metadata", None);
        assert!(cmd.is_none(), "ambiguous fix should not route");
    }

    #[test]
    fn ambiguous_question_does_not_route() {
        let cmd = route_message("what genres are in this library?", None);
        assert!(cmd.is_none());
    }

    /// Unambiguous "set the missing genre" without a value
    /// Should route because it's clearly about genre, but value will be None
    /// so the caller must get it (from LLM or follow-up).
    #[test]
    fn fix_missing_routes_but_needs_value() {
        let cmd = route_message("fix the missing genre", None).unwrap();
        assert_eq!(cmd.intent, IntentKind::SetMissing);
        assert_eq!(cmd.field.as_deref(), Some("genre"));
        assert!(cmd.value.is_none(), "no value specified yet");
        assert!(cmd.only_if_missing);
    }

    // ── scope resolution ───────────────────────────────────────────

    #[test]
    fn resolve_missing_scope_finds_empty_tracks() {
        let input = AssistantSendInput {
            tracks: vec![
                serde_json::json!({"path": "/a.mp3", "genre": "Rock"}),
                serde_json::json!({"path": "/b.mp3", "genre": ""}),
                serde_json::json!({"path": "/c.mp3"}), // no genre
            ],
            ..Default::default()
        };
        let cmd = route_message("set genre to Pop where missing", None).unwrap();
        let paths = resolve_scope(&cmd, &input, None);
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"/b.mp3".to_string()));
        assert!(paths.contains(&"/c.mp3".to_string()));
    }
}
