//! User-learned artist alias persistence shared by audit and auto-tag.

use std::fs;
use std::io;
use std::path::Path;

pub fn save_alias(path: &Path, hint: &str, alias: &str) -> Result<(), io::Error> {
    let hint = hint.trim().to_lowercase();
    let alias = alias.trim();
    if hint.is_empty() || alias.is_empty() || hint == alias.to_lowercase() {
        return Ok(());
    }
    let mut aliases = fs::read_to_string(path)
        .ok()
        .and_then(|text| {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&text).ok()
        })
        .unwrap_or_default();
    let existing = aliases
        .entry(hint)
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(existing) = existing.as_array_mut() else {
        return Ok(());
    };
    if existing.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case(alias))
    }) {
        return Ok(());
    }
    existing.push(serde_json::Value::String(alias.to_string()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(&aliases).map_err(io::Error::other)?;
    fs::write(path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn save_alias_normalizes_keys_deduplicates_and_preserves_existing_entries() {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "soundrobe-aliases-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("artist-aliases.json");
        fs::write(&path, r#"{"existing":["Keep"]}"#).unwrap();

        save_alias(&path, " 周杰倫 ", " Jay Chou ").unwrap();
        save_alias(&path, "周杰倫", "jay chou").unwrap();

        let aliases: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(aliases["existing"][0], "Keep");
        assert_eq!(aliases["周杰倫"], serde_json::json!(["Jay Chou"]));
        fs::remove_dir_all(root).unwrap();
    }
}
