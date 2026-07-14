//! `~/.auto-tagger/config.yaml` — flat parser, env precedence, save-with
//! preservation, and renderer-facing redaction.
//!
//! Faithful port of `electron/handlers/auto-tag.ts` (loadConfig / saveConfig /
//! getConfig / formatYamlValue / CONFIG_KEY_MAP). Parity invariants this module
//! keeps so both runtimes can use the same file during migration:
//!   - flat `key: value` parsing only (no nested structures, no YAML dep);
//!   - blank and `#` comment lines are skipped on read but preserved on write;
//!   - matching surrounding `"…"` / `'…"` quotes are stripped on read;
//!   - environment variables override the file (highest priority);
//!   - saving a key rewrites just its value, preserves all other lines (incl.
//!     comments and unknown keys) and order, and appends if absent;
//!   - unknown camelCase keys are ignored (no partial write).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Resolved app configuration. Fields mirror `AutoTagConfig`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoTagConfig {
    pub llm_api_key: Option<String>,
    pub llm_model: Option<String>,
    pub dataset_path: Option<String>,
    pub cache_path: Option<String>,
    pub discogs_token: Option<String>,
    pub remote_lookup_enabled: Option<bool>,
    pub discogs_enabled: Option<bool>,
    pub debug: Option<bool>,
    pub lyrics_download_enabled: Option<bool>,
    pub lyrics_api_url: Option<String>,
    pub theaudiodb_api_key: Option<String>,
    pub chinese_script: Option<String>,
}

/// Environment accessor used by [`load_from`]. Tests supply [`EnvMap`]; the
/// runtime uses [`ProcessEnv`] so `HOME` and `AUTO_TAG_*` precedence is honored
/// without touching the global process env from unit tests.
pub trait Env {
    fn get(&self, name: &str) -> Option<String>;
}

/// Real process environment.
pub struct ProcessEnv;
impl Env for ProcessEnv {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// In-memory env for tests.
#[derive(Debug, Default, Clone)]
pub struct EnvMap(HashMap<String, String>);
impl EnvMap {
    pub fn new() -> Self {
        Self(HashMap::new())
    }
    pub fn set(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.0.insert(k.into(), v.into());
        self
    }
}
impl Env for EnvMap {
    fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

/// `~/.auto-tagger/config.yaml` path (electron returns this single path).
pub fn config_file_path(home: &Path) -> PathBuf {
    home.join(".auto-tagger").join("config.yaml")
}

/// Load config from a flat YAML body plus environment overrides. Mirrors
/// `loadConfig()`: file first, env overrides afterwards; defaults
/// `remote_lookup_enabled` and `discogs_enabled` to `true`.
pub fn load_from(text: &str, env: &dyn Env) -> AutoTagConfig {
    let mut config = AutoTagConfig {
        remote_lookup_enabled: Some(true),
        discogs_enabled: Some(true),
        ..Default::default()
    };

    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        let key = trimmed[..colon].trim();
        let mut value = trimmed[colon + 1..].trim().to_string();
        if value.len() >= 2 {
            let bytes = value.as_bytes();
            let quoted = (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
                || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'');
            if quoted {
                value = value[1..value.len() - 1].to_string();
            }
        }
        apply_yaml_key(&mut config, key, &value);
    }

    // Environment overrides (highest priority), exactly as in loadConfig.
    if let Some(v) = env.get("LLM_API_KEY") {
        config.llm_api_key = Some(v);
    }
    if let Some(v) = env.get("LLM_MODEL") {
        config.llm_model = Some(v);
    }
    if let Some(v) = env.get("AUTO_TAG_DISCOGS_TOKEN") {
        config.discogs_token = Some(v);
    }
    if env.get("AUTO_TAG_REMOTE_LOOKUP").as_deref() == Some("false") {
        config.remote_lookup_enabled = Some(false);
    }
    if env.get("AUTO_TAG_DISCOGS_ENABLED").as_deref() == Some("false") {
        config.discogs_enabled = Some(false);
    }
    if env.get("AUTO_TAG_DEBUG").as_deref() == Some("true") {
        config.debug = Some(true);
    }
    if env.get("AUTO_TAG_LYRICS_DOWNLOAD_ENABLED").as_deref() == Some("false") {
        config.lyrics_download_enabled = Some(false);
    }
    if let Some(v) = env.get("AUTO_TAG_LYRICS_API_URL") {
        config.lyrics_api_url = Some(v);
    }
    if let Some(v) = env.get("AUTO_TAG_CHINESE_SCRIPT") {
        config.chinese_script = Some(v);
    }
    if let Some(v) = env.get("THEAUDIODB_API_KEY") {
        config.theaudiodb_api_key = Some(v);
    }

    config
}

fn parse_bool_or_null(v: &str) -> Option<bool> {
    match v {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn apply_yaml_key(config: &mut AutoTagConfig, key: &str, value: &str) {
    match key {
        "llm_api_key" => config.llm_api_key = Some(value.to_string()),
        "llm_model" => config.llm_model = Some(value.to_string()),
        "discogs_token" => config.discogs_token = Some(value.to_string()),
        "dataset_path" => config.dataset_path = Some(value.to_string()),
        "remote_lookup_enabled" => {
            if let Some(b) = parse_bool_or_null(value) {
                config.remote_lookup_enabled = Some(b);
            }
        }
        "discogs_enabled" => {
            if let Some(b) = parse_bool_or_null(value) {
                config.discogs_enabled = Some(b);
            }
        }
        "debug" => {
            if let Some(b) = parse_bool_or_null(value) {
                config.debug = Some(b);
            }
        }
        "lyrics_download_enabled" => {
            if let Some(b) = parse_bool_or_null(value) {
                config.lyrics_download_enabled = Some(b);
            }
        }
        "lyrics_api_url" => config.lyrics_api_url = Some(value.to_string()),
        "theaudiodb_api_key" => config.theaudiodb_api_key = Some(value.to_string()),
        "chinese_script" => config.chinese_script = Some(value.to_string()),
        _ => {} // unknown keys ignored, never partially written
    }
}

/// Map a renderer camelCase config key to its YAML key (CONFIG_KEY_MAP).
pub fn yaml_key_for(camel_key: &str) -> Option<&'static str> {
    match camel_key {
        "llmApiKey" => Some("llm_api_key"),
        "llmModel" => Some("llm_model"),
        "discogsToken" => Some("discogs_token"),
        "remoteLookupEnabled" => Some("remote_lookup_enabled"),
        "discogsEnabled" => Some("discogs_enabled"),
        "debug" => Some("debug"),
        "lyricsDownloadEnabled" => Some("lyrics_download_enabled"),
        "lyricsApiUrl" => Some("lyrics_api_url"),
        "theAudioDbApiKey" => Some("theaudiodb_api_key"),
        "chineseScript" => Some("chinese_script"),
        _ => None,
    }
}

/// Format a value for YAML, matching `formatYamlValue`: booleans, `null`,
/// quoted (`"…"` with escaped inner quotes) when the string contains
/// whitespace, `:`, or `#`, else raw.
pub fn format_yaml_value(value: &Value) -> String {
    match value {
        Value::Bool(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Value::Null => "null".to_string(),
        other => {
            let s = match other {
                Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            if s.chars().any(|c| c.is_whitespace() || c == ':' || c == '#') {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s
            }
        }
    }
}

/// Rewrite a single key's value in a flat YAML body, preserving every other
/// line (comments, unknown keys, order) and appending the key if absent.
/// Mirrors the line map in `saveConfig`.
pub fn apply_key(text: &str, yaml_key: &str, formatted_value: &str) -> String {
    let mut lines: Vec<String> = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(str::to_string).collect()
    };
    let mut found = false;
    for line in lines.iter_mut() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        let existing_key = trimmed[..colon].trim();
        if existing_key == yaml_key {
            found = true;
            *line = format!("{yaml_key}: {formatted_value}");
        }
    }
    if !found {
        lines.push(format!("{yaml_key}: {formatted_value}"));
    }
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Persist a renderer camelCase key to the config file in place. Creates the
/// parent directory. Unknown keys are ignored (matches Electron's early return).
pub fn save_config(home: &Path, camel_key: &str, value: &Value) -> std::io::Result<()> {
    let Some(yaml_key) = yaml_key_for(camel_key) else {
        return Ok(()); // unknown key — skip, never partially write
    };
    let path = config_file_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let updated = apply_key(&existing, yaml_key, &format_yaml_value(value));
    fs::write(&path, updated)
}

fn mask(s: &Option<String>) -> Value {
    match s {
        Some(v) if !v.is_empty() => {
            let last4 = v
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            Value::String(format!("****{last4}"))
        }
        _ => Value::Null,
    }
}

/// Renderer-facing redacted view, matching `getConfig()` exactly (masked
/// secrets, `null` when absent, `false` default for bools, `null` for
/// optional strings). Note: Electron emits the raw `llmModel` (possibly
/// `undefined`, serialized as absent) — we emit `null` for the absent case so
/// the renderer's "not set" semantics hold; parity is revisited when the
/// config command is wired.
pub fn redacted(config: &AutoTagConfig) -> Value {
    json!({
        "llmApiKey": mask(&config.llm_api_key),
        "llmModel": config.llm_model.clone().map(Value::String).unwrap_or(Value::Null),
        "discogsToken": mask(&config.discogs_token),
        "remoteLookupEnabled": config.remote_lookup_enabled.unwrap_or(true),
        "discogsEnabled": config.discogs_enabled.unwrap_or(true),
        "debug": config.debug.unwrap_or(false),
        "lyricsDownloadEnabled": config.lyrics_download_enabled.unwrap_or(false),
        "lyricsApiUrl": config.lyrics_api_url.clone().map(Value::String).unwrap_or(Value::Null),
        "theAudioDbApiKey": mask(&config.theaudiodb_api_key),
        "chineseScript": config.chinese_script.clone().map(Value::String).unwrap_or(Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Intent: defaults must enable remote + Discogs lookups (opt-in-by-default
    /// safety guard) even with an empty file.
    #[test]
    fn load_defaults_enable_remote_and_discogs() {
        let c = load_from("", &EnvMap::new());
        assert_eq!(c.remote_lookup_enabled, Some(true));
        assert_eq!(c.discogs_enabled, Some(true));
        assert_eq!(c.debug, None);
    }

    /// Intent: comments and blank lines must not parse, and quoted values are
    /// unwrapped so a quoted secret is not stored with surrounding quotes.
    #[test]
    fn parse_skips_comments_and_unwraps_quotes() {
        let text = "# top comment\n\nllm_api_key: \"secret-key\"\ndebug: true\n";
        let c = load_from(text, &EnvMap::new());
        assert_eq!(c.llm_api_key.as_deref(), Some("secret-key"));
        assert_eq!(c.debug, Some(true));
    }

    /// Intent: env vars override the file — providers must read the live key,
    /// not a stale on-disk value, when the operator exports a different one.
    /// Specifically `AUTO_TAG_REMOTE_LOOKUP=false` disables even if the file
    /// omits it; `LLM_API_KEY` wins outright.
    #[test]
    fn env_overrides_file() {
        let text = "llm_api_key: file-key\nremote_lookup_enabled: true\n";
        let env = EnvMap::new()
            .set("LLM_API_KEY", "env-key")
            .set("AUTO_TAG_REMOTE_LOOKUP", "false");
        let c = load_from(text, &env);
        assert_eq!(c.llm_api_key.as_deref(), Some("env-key"));
        assert_eq!(c.remote_lookup_enabled, Some(false));
    }

    #[test]
    fn env_debug_only_flips_on_true() {
        let c = load_from("", &EnvMap::new().set("AUTO_TAG_DEBUG", "false"));
        assert_eq!(c.debug, None);
        let c = load_from("", &EnvMap::new().set("AUTO_TAG_DEBUG", "true"));
        assert_eq!(c.debug, Some(true));
    }

    /// Intent: redaction never leaks a secret — only the last 4 chars appear,
    /// and an absent secret is `null`, not an empty string.
    #[test]
    fn redaction_masks_last_four_and_nulls_absent() {
        let mut c = AutoTagConfig {
            llm_api_key: Some("sk-or-v1-abcd1234".to_string()),
            discogs_token: None,
            remote_lookup_enabled: Some(true),
            discogs_enabled: Some(true),
            ..Default::default()
        };
        let r = redacted(&c);
        assert_eq!(r["llmApiKey"], json!("****1234"));
        assert_eq!(r["discogsToken"], Value::Null);
        assert_eq!(r["remoteLookupEnabled"], json!(true));
        c.llm_api_key = Some("k".to_string()); // shorter than 4 chars
        assert_eq!(redacted(&c)["llmApiKey"], json!("****k"));
    }

    /// Intent: `format_yaml_value` quotes strings with whitespace/`:`/`#` and
    /// escapes inner quotes, but keeps simple values raw — so a model id with a
    /// slash isn't needlessly quoted while a URL with `:` is.
    #[test]
    fn format_yaml_value_quoting_rules() {
        assert_eq!(
            format_yaml_value(&json!("meta-llama/70b")),
            "meta-llama/70b"
        );
        assert_eq!(format_yaml_value(&json!("a b")), "\"a b\"");
        assert_eq!(format_yaml_value(&json!("https://x")), "\"https://x\"");
        assert_eq!(format_yaml_value(&json!("tag#1")), "\"tag#1\"");
        // Quotes alone don't trigger quoting (no whitespace/`:`/`#`), matching Electron's regex.
        assert_eq!(format_yaml_value(&json!("\"hi\"")), "\"hi\"");
        // When whitespace forces quoting, inner quotes are escaped.
        assert_eq!(format_yaml_value(&json!("a \"b\" c")), "\"a \\\"b\\\" c\"");
        assert_eq!(format_yaml_value(&json!(true)), "true");
        assert_eq!(format_yaml_value(&json!(false)), "false");
        assert_eq!(format_yaml_value(&Value::Null), "null");
    }

    /// Intent: saving a key must preserve comments, unknown keys, and line
    /// order, and must update only the matching line — so hand-edited config
    /// isn't reformatted away (Electron's flat save preserves the file body).
    #[test]
    fn save_preserves_comments_order_and_unknown_keys() {
        let text = "# my notes\nllm_api_key: old\nunknown_thing: keep\ndebug: true\n";
        let out = apply_key(text, "llm_api_key", "new-secret");
        // The comment and unknown key survive untouched and in place.
        assert!(out.contains("# my notes"));
        assert!(out.contains("unknown_thing: keep"));
        assert!(out.contains("llm_api_key: new-secret"));
        assert!(!out.contains("old"));
        // Order preserved: the comment is still first.
        assert_eq!(out.lines().next().unwrap(), "# my notes");
    }

    /// Intent: saving a key that isn't present appends it rather than muting it.
    #[test]
    fn save_appends_missing_key() {
        let out = apply_key("debug: true\n", "llm_model", "gpt-x");
        assert!(out.ends_with("llm_model: gpt-x\n"));
        assert!(out.contains("debug: true"));
    }

    /// Intent: an unknown renderer key is a no-op — the file is never touched,
    /// because `CONFIG_KEY_MAP` dropping an unknown key must not corrupt the
    /// config by writing a bogus YAML key.
    #[test]
    fn save_ignores_unknown_key() {
        let dir = std::env::temp_dir().join(format!(
            "auto-tag-cfg-{}",
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".auto-tagger")).unwrap();
        std::fs::write(
            dir.join(".auto-tagger").join("config.yaml"),
            "debug: true\n",
        )
        .unwrap();
        save_config(&dir, "notARealKey", &json!("x")).unwrap();
        let written =
            std::fs::read_to_string(dir.join(".auto-tagger").join("config.yaml")).unwrap();
        assert_eq!(written, "debug: true\n");
    }

    #[test]
    fn save_round_trip_through_file() {
        let dir = std::env::temp_dir().join(format!(
            "auto-tag-cfg2-{}",
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .unwrap()
                .as_nanos()
        ));
        save_config(&dir, "llmApiKey", &json!("sk-1")).unwrap();
        let written = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert!(written.contains("llm_api_key: sk-1"));
        // Load it back via the parser.
        let c = load_from(&written, &EnvMap::new());
        assert_eq!(c.llm_api_key.as_deref(), Some("sk-1"));
    }
}
