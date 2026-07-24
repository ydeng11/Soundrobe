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
use std::sync::{Arc, Mutex, MutexGuard};

/// Resolved app configuration. Fields mirror `AutoTagConfig`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoTagConfig {
    pub llm_api_key: Option<String>,
    pub llm_model: Option<String>,
    pub llm_provider: Option<String>,
    pub llm_base_url: Option<String>,
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
    /// Maximum concurrent folder workers during batch track writes.
    /// Higher values (e.g. 8) speed up local SSD/NVMe writes;
    /// lower values (default 2) avoid I/O thrashing on external/spinning
    /// volumes where parallel reads+writes degrade per-file throughput.
    /// Set to `0` or omit for the default (2).
    pub write_concurrency: Option<usize>,
}

/// Environment accessor used by [`load_from`] and [`ConfigState`]. Tests
/// supply [`EnvMap`]; the runtime uses [`ProcessEnv`] so `HOME` and
/// `AUTO_TAG_*` precedence is honored without touching the global process env
/// from unit tests. `Send + Sync` so `ConfigState` can hold an `Arc<dyn Env>`
/// and be `tauri::manage`d across threads.
pub trait Env: Send + Sync {
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
    if let Some(v) = env.get("LLM_PROVIDER") {
        config.llm_provider = Some(v);
    }
    if let Some(v) = env.get("LLM_BASE_URL") {
        config.llm_base_url = Some(v);
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
    if let Some(v) = env.get("AUTO_TAG_WRITE_CONCURRENCY") {
        if let Ok(n) = v.parse::<usize>() {
            config.write_concurrency = Some(n);
        }
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
        "llm_provider" => config.llm_provider = Some(value.to_string()),
        "llm_base_url" => config.llm_base_url = Some(value.to_string()),
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
        "chinese_script" => {
            if value == "null" || value.is_empty() {
                config.chinese_script = None;
            } else {
                config.chinese_script = Some(value.to_string());
            }
        }
        "write_concurrency" => {
            if let Ok(n) = value.parse::<usize>() {
                config.write_concurrency = Some(n);
            }
        }
        _ => {} // unknown keys ignored, never partially written
    }
}

/// Resolve the effective write concurrency from config + env.
/// Returns `None` when neither the file nor the env specifies a value,
/// meaning the caller should use its built-in default (currently 2).
pub fn resolve_write_concurrency(home: &Path) -> Option<usize> {
    let text = std::fs::read_to_string(config_file_path(home)).unwrap_or_default();
    load_from(&text, &ProcessEnv).write_concurrency
}

/// Map a renderer camelCase config key to its YAML key (CONFIG_KEY_MAP).
pub fn yaml_key_for(camel_key: &str) -> Option<&'static str> {
    match camel_key {
        "llmApiKey" => Some("llm_api_key"),
        "llmModel" => Some("llm_model"),
        "llmProvider" => Some("llm_provider"),
        "llmBaseUrl" => Some("llm_base_url"),
        "discogsToken" => Some("discogs_token"),
        "remoteLookupEnabled" => Some("remote_lookup_enabled"),
        "discogsEnabled" => Some("discogs_enabled"),
        "debug" => Some("debug"),
        "lyricsDownloadEnabled" => Some("lyrics_download_enabled"),
        "lyricsApiUrl" => Some("lyrics_api_url"),
        "theAudioDbApiKey" => Some("theaudiodb_api_key"),
        "chineseScript" => Some("chinese_script"),
        "writeConcurrency" => Some("write_concurrency"),
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
/// line (comments, unknown keys, order, and any leading whitespace) and
/// appending the key if absent. Mirrors Electron's
/// `line.replace(/:.+/, ": " + formattedValue)`: only the text from the first
/// `:` onward is replaced — the key name and its indentation survive intact.
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
            // Preserve the indentation and key text before the colon; replace
            // only from the first `:` (exactly `line.replace(/:.+/, ...)`).
            let prefix = &line[..line.find(':').unwrap()];
            *line = format!("{prefix}: {formatted_value}");
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
    let key_configured = config.llm_api_key.as_deref().filter(|k| !k.is_empty()).is_some();
    json!({
        "llmApiKey": mask(&config.llm_api_key),
        "llmApiKeyConfigured": key_configured,
        "llmModel": config.llm_model.clone().map(Value::String).unwrap_or(Value::Null),
        "llmProvider": config.llm_provider.clone().map(Value::String).unwrap_or(Value::Null),
        "llmBaseUrl": config.llm_base_url.clone().map(Value::String).unwrap_or(Value::Null),
        "discogsToken": mask(&config.discogs_token),
        "remoteLookupEnabled": config.remote_lookup_enabled.unwrap_or(true),
        "discogsEnabled": config.discogs_enabled.unwrap_or(true),
        "debug": config.debug.unwrap_or(false),
        "lyricsDownloadEnabled": config.lyrics_download_enabled.unwrap_or(false),
        "lyricsApiUrl": config.lyrics_api_url.clone().map(Value::String).unwrap_or(Value::Null),
        "theAudioDbApiKey": mask(&config.theaudiodb_api_key),
        "chineseScript": config.chinese_script.clone().map(Value::String).unwrap_or(Value::Null),
        "writeConcurrency": config.write_concurrency.map(Value::from).unwrap_or(Value::Null),
    })
}

/// Load config from the on-disk YAML at `home/.auto-tagger/config.yaml` plus an
/// env. Missing/unreadable file yields the empty text (defaults), matching
/// Electron's behavior when no config exists yet.
fn load_from_disk(home: &Path, env: &dyn Env) -> AutoTagConfig {
    let text = fs::read_to_string(config_file_path(home)).unwrap_or_default();
    load_from(&text, env)
}

/// Managed state holding the live app config, mirroring the role of
/// `TaskManager`'s config in `electron/handlers/auto-tag.ts`: loaded once at
/// startup from `config.yaml` + the process environment, and refreshed after a
/// `set_config` write. Held behind a `Mutex` so Tauri commands read it
/// concurrently without holding a SQLite/network lock.
pub struct ConfigState {
    home: PathBuf,
    env: Arc<dyn Env>,
    inner: Arc<Mutex<AutoTagConfig>>,
    /// Serialises concurrent read-modify-write of the YAML file.
    /// `Promise.all` in SettingsModal.handleSave fires every `config_set`
    /// simultaneously; without this lock, independent read-modify-write
    /// cycles in `set` overwrite one another, losing keys.
    write_lock: Arc<Mutex<()>>,
}

impl ConfigState {
    /// Load config from `~/.auto-tagger/config.yaml` + the real process env.
    pub fn init(home: PathBuf) -> Self {
        Self::init_with_env(home, Arc::new(ProcessEnv))
    }

    /// Load config from a given home dir + an injected env (tests).
    pub fn init_with_env(home: PathBuf, env: Arc<dyn Env>) -> Self {
        let config = load_from_disk(&home, env.as_ref());
        Self {
            home,
            env,
            inner: Arc::new(Mutex::new(config)),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Renderer-facing redacted snapshot (matches `getConfig()`). On a poisoned
    /// lock (a prior panic during config access), Electron's `getConfig()`
    /// catches and returns `{}`; we mirror that by logging and returning an empty
    /// JSON object, so `config_get` never panics and the renderer always gets a
    /// shape-consistent object.
    pub fn redacted(&self) -> Value {
        match self.inner.lock() {
            Ok(guard) => redacted(&guard),
            Err(e) => {
                tracing::error!("config mutex poisoned: {e}");
                json!({})
            }
        }
    }

    /// Raw (unredacted) snapshot for main-process internal use. On a poisoned
    /// lock, return the default config (no secrets, lookups opt-in) rather than
    /// panicking — a calling slice must not take the shell down.
    pub fn raw(&self) -> AutoTagConfig {
        match self.inner.lock() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                tracing::error!("config mutex poisoned: {e}");
                AutoTagConfig::default()
            }
        }
    }

    pub fn alias_file_path(&self) -> PathBuf {
        self.home.join(".auto-tagger").join("artist-aliases.json")
    }

    /// Reload config from disk + env (matches `refreshConfig()`). On a poisoned
    /// lock (a prior panic while holding it), the live config cannot be updated
    /// — std::sync `Mutex` poison cannot be cleared by `into_inner` (the next
    /// `lock()` still fails). We mirror Electron's tolerance by logging loudly and
    /// leaving the live state unchanged (degrading to the restart-loaded values
    /// until the app is restarted), never panicking. The on-disk file is still
    /// correct, so a restart picks it up.
    pub fn refresh(&self) {
        let config = load_from_disk(&self.home, self.env.as_ref());
        match self.inner.lock() {
            Ok(mut guard) => *guard = config,
            Err(_) => {
                tracing::error!(
                    "config mutex poisoned; live config left unchanged. Restart to recover."
                );
            }
        }
    }

    /// Write a renderer camelCase key to disk and refresh the live config
    /// (matches the `config:set` handler: `saveConfig` + `refreshConfig`). Never
    /// returns an error to the caller — Electron's handler catches and logs — so
    /// the renderer's `setConfig` never rejects. A failed write is logged via
    /// `tracing` and the live config is left untouched.
    ///
    /// **Serialised**: holds `write_lock` across the read-modify-write **and**
    /// the subsequent `refresh` so that concurrent `Promise.all` calls from the
    /// renderer do not overwrite each other (each starts with `read_to_string`,
    /// applies one key, and writes back — concurrent calls lose keys).
    ///
    /// The lock is held across `refresh()` too because otherwise an earlier
    /// thread's `refresh` could interleave after a later thread's `save_config`
    /// and produce a stale in-memory snapshot.  Since `refresh` acquires
    /// `inner.lock()` independently there is no deadlock risk (no code path
    /// acquires `write_lock` while holding `inner.lock()`).
    pub fn set(&self, camel_key: &str, value: &Value) {
        let _guard: MutexGuard<'_, ()> = match self.write_lock.lock() {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(
                    "config write-lock poisoned, skipping save for {camel_key}: {e}"
                );
                return;
            }
        };
        if let Err(e) = save_config(&self.home, camel_key, value) {
            tracing::warn!("failed to save config key {camel_key}: {e}");
            return;
        }
        // `refresh` already handles a poisoned mutex without panicking.
        self.refresh();
        // _guard dropped here — write-lock released after the in-memory state
        // is updated, guaranteeing the live config reflects the write before
        // the next queued writer starts.
    }
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

    /// Intent (from test/handlers/config.test.ts `updates an existing key`):
    /// updating one key must leave the other keys and order intact.
    #[test]
    fn updates_existing_key_preserving_others_fixture() {
        let text = "llm_model: old-model\nremote_lookup_enabled: true\n";
        let out = apply_key(text, "llm_model", "new-model");
        assert!(out.contains("llm_model: new-model"));
        assert!(out.contains("remote_lookup_enabled: true"));
        assert!(!out.contains("old-model"));
    }

    /// Intent (from config.test.ts `maps JS camelCase to YAML snake_case`):
    /// the camelCase->yaml key map and bool formatting underpin the renderer's
    /// `setConfig` round-trip.
    #[test]
    fn camel_to_yaml_map_and_bool_format_fixture() {
        let key = yaml_key_for("discogsEnabled").unwrap();
        assert_eq!(key, "discogs_enabled");
        let text = apply_key("", key, &format_yaml_value(&json!(false)));
        assert_eq!(text, "discogs_enabled: false\n");
    }

    /// Intent: leading indentation must survive a value update — Electron's
    /// `line.replace(/:.+/, ...)` keeps the whitespace/key prefix, so a
    /// hand-indented file keeps its layout.
    #[test]
    fn preserves_leading_indentation_on_update() {
        let text = "  llm_model: old\n    discogs_token: tok\n";
        let out = apply_key(text, "llm_model", "new");
        assert!(out.contains("  llm_model: new\n"), "got: {out}");
        assert!(out.contains("    discogs_token: tok\n"));
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

    fn cfg_home() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "auto-tag-cfgstate-{}-{}",
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .unwrap()
                .as_nanos(),
            seq
        ))
    }

    #[test]
    fn config_state_init_loads_disk_and_env() {
        let home = cfg_home();
        fs::create_dir_all(home.join(".auto-tagger")).unwrap();
        fs::write(config_file_path(&home), "llm_model: gpt-4\ndebug: true\n").unwrap();
        let env = EnvMap::new().set("LLM_API_KEY", "env-override");
        let state = ConfigState::init_with_env(home, Arc::new(env));
        // Env wins over file for llm_api_key; file value read for llm_model/debug.
        let raw = state.raw();
        assert_eq!(raw.llm_api_key.as_deref(), Some("env-override"));
        assert_eq!(raw.llm_model.as_deref(), Some("gpt-4"));
        assert_eq!(raw.debug, Some(true));
    }

    #[test]
    fn config_state_set_writes_and_refreshes() {
        let home = cfg_home();
        let state = ConfigState::init_with_env(home.clone(), Arc::new(EnvMap::new()));
        // File doesn't exist yet -> set creates it and refreshes the live config.
        state.set("llmApiKey", &json!("sk-or-v1-1234567890"));
        assert!(config_file_path(&home).exists());
        assert_eq!(
            state.raw().llm_api_key.as_deref(),
            Some("sk-or-v1-1234567890")
        );
        // A subsequent set updates only that key.
        state.set("debug", &json!(true));
        let raw = state.raw();
        assert_eq!(raw.debug, Some(true));
        assert_eq!(raw.llm_api_key.as_deref(), Some("sk-or-v1-1234567890"));
    }

    #[test]
    fn config_state_refresh_picks_up_external_file_change() {
        let home = cfg_home();
        fs::create_dir_all(home.join(".auto-tagger")).unwrap();
        fs::write(config_file_path(&home), "llm_model: old\n").unwrap();
        let state = ConfigState::init_with_env(home.clone(), Arc::new(EnvMap::new()));
        assert_eq!(state.raw().llm_model.as_deref(), Some("old"));
        // External edit (another process / the renderer saves via a text editor).
        fs::write(config_file_path(&home), "llm_model: new\n").unwrap();
        state.refresh();
        assert_eq!(state.raw().llm_model.as_deref(), Some("new"));
    }

    /// Normalized Electron-vs-Rust redaction fixture. The expected JSON is the
    /// contract for `config:get`'s response shape; the matching Vitest test
    /// (`test/handlers/config.test.ts` redaction_fixture) asserts Electron's
    /// `getConfig()` formula produces the SAME object from the same on-disk
    /// file. Both runtimes must agree exactly.
    #[test]
    fn redacted_fixture_matches_normalized_contract() {
        let home = cfg_home();
        fs::create_dir_all(home.join(".auto-tagger")).unwrap();
        fs::write(
            config_file_path(&home),
            "llm_api_key: sk-or-v1-1234567890\n\
             llm_model: gpt-4\n\
             discogs_token: mytoken1234\n\
             debug: true\n\
             lyrics_api_url: https://lr.example/api\n\
             chinese_script: traditional\n",
        )
        .unwrap();
        let state = ConfigState::init_with_env(home, Arc::new(EnvMap::new()));

        let expected = json!({
            "llmApiKey": "****7890",
            "llmApiKeyConfigured": true,
            "llmModel": "gpt-4",
            "llmProvider": null,
            "llmBaseUrl": null,
            "discogsToken": "****1234",
            "remoteLookupEnabled": true,
            "discogsEnabled": true,
            "debug": true,
            "lyricsDownloadEnabled": false,
            "lyricsApiUrl": "https://lr.example/api",
            "theAudioDbApiKey": null,
            "chineseScript": "traditional",
            "writeConcurrency": null
        });
        assert_eq!(state.redacted(), expected);
    }

    /// Intent: a poisoned lock must not take `config_get` down — Electron's
    /// `getConfig()` catches and returns {}; Rust mirrors that by returning `{}`
    /// (redacted) or the default config (raw) and leaving the live state
    /// unchanged on refresh — never panicking, so one panic elsewhere can't
    /// crash the shell or the renderer's settings panel.
    #[test]
    fn config_state_survives_poisoned_lock() {
        let home = cfg_home();
        let state = ConfigState::init_with_env(home.clone(), Arc::new(EnvMap::new()));
        let inner = state.inner.clone();
        let h = std::thread::spawn(move || {
            let _g = inner.lock().unwrap();
            panic!("intentionally poison the config mutex");
        })
        .join();
        assert!(
            h.is_err(),
            "helper thread should have panicked to poison the lock"
        );
        assert_eq!(
            state.redacted(),
            json!({}),
            "redacted degrades to {{}} on poison"
        );
        assert_eq!(
            state.raw(),
            AutoTagConfig::default(),
            "raw degrades to default on poison"
        );
        // refresh leaves the live state unchanged rather than silently writing
        // into a dead lock (std::Mutex poison cannot be cleared).
        fs::create_dir_all(home.join(".auto-tagger")).unwrap();
        fs::write(config_file_path(&home), "llm_model: fresh\n").unwrap();
        state.refresh();
        assert_eq!(
            state.raw(),
            AutoTagConfig::default(),
            "poison is permanent for the instance; live config degraded (restart recovers)"
        );
        assert_eq!(state.redacted(), json!({}));
    }

    /// Intent: concurrent writes preserve every key (the `Promise.all` scenario
    /// in SettingsModal.handleSave).  Without the write-lock in
    /// [`ConfigState::set`], each `set` call does an independent
    /// read-modify-write of the same file, so the last writer wins and earlier
    /// keys are silently lost.
    #[test]
    fn concurrent_writes_preserve_all_keys() {
        let home = cfg_home();
        let state = ConfigState::init_with_env(home.clone(), Arc::new(EnvMap::new()));

        // Use scoped threads so `&state` is accessible (the closure only needs
        // a shared reference, not `'static + Send`). Scoped threads guarantee
        // all spawned threads complete before the scope ends.
        std::thread::scope(|s| {
            s.spawn(|| {
                state.set("llmModel", &json!("gpt-4"));
            });
            s.spawn(|| {
                state.set("debug", &json!(true));
            });
            s.spawn(|| {
                state.set("chineseScript", &json!("simplified"));
            });
        });

        let raw = state.raw();
        assert_eq!(
            raw.llm_model.as_deref(),
            Some("gpt-4"),
            "llm_model must survive concurrent writes"
        );
        assert_eq!(
            raw.debug,
            Some(true),
            "debug must survive concurrent writes"
        );
        assert_eq!(
            raw.chinese_script.as_deref(),
            Some("simplified"),
            "chinese_script must survive concurrent writes"
        );
    }

    /// Intent: when `chinese_script` in the YAML is `null` (written by the
    /// renderer when the user selects "Default (no conversion)"), the parser
    /// must produce `None` rather than `Some("null")` so that
    /// `effective_chinese_target` returns `None` in release builds.
    #[test]
    fn chinese_script_null_is_parsed_as_none() {
        let text = "chinese_script: null\n";
        let c = load_from(text, &EnvMap::new());
        assert_eq!(
            c.chinese_script, None,
            "chinese_script: null should yield None, not Some('null')"
        );
    }

    /// Intent: empty-string chinese_script also yields None (defensive).
    #[test]
    fn chinese_script_empty_is_parsed_as_none() {
        let text = "chinese_script: ''\n";
        let c = load_from(text, &EnvMap::new());
        assert_eq!(
            c.chinese_script, None,
            "chinese_script: '' should yield None"
        );
    }
}
