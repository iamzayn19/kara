//! `kara config get/set`: read effective values and edit the user config file
//! in place, preserving comments and layout. Every edit is validated against
//! the schema before it is written.

use crate::config::Config;
use std::path::Path;

/// Look up a dotted key (e.g. `permissions.mode`) in a config.
pub fn get(config: &Config, key: &str) -> anyhow::Result<String> {
    let value = toml::Value::try_from(config)?;
    let mut cur = &value;
    for part in key.split('.') {
        cur = cur
            .get(part)
            .ok_or_else(|| anyhow::anyhow!("unknown configuration key `{key}`"))?;
    }
    Ok(match cur {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(_) => toml::to_string_pretty(cur)?.trim_end().to_string(),
        other => other.to_string(),
    })
}

/// Parse a command-line value: TOML literals (`true`, `8`, `["a"]`) keep
/// their type, anything else is a string.
fn parse_value(raw: &str) -> toml_edit::Value {
    match raw.trim().parse::<toml_edit::Value>() {
        Ok(v) if !matches!(v, toml_edit::Value::InlineTable(_)) => v,
        _ => toml_edit::Value::from(raw),
    }
}

/// Return `text` with `key` set to `raw`. Fails if the result is not a valid
/// Kara configuration (unknown key, wrong type, invalid value).
pub fn set_in_text(text: &str, key: &str, raw: &str) -> anyhow::Result<String> {
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    let parts: Vec<&str> = key.split('.').collect();
    if parts.len() < 2 || parts.iter().any(|p| p.is_empty()) {
        anyhow::bail!("use a dotted key such as permissions.mode");
    }
    let (last, tables) = parts.split_last().expect("at least two parts");
    let mut table = doc.as_table_mut();
    for t in tables {
        if !table.contains_key(t) {
            table.insert(t, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        table = table[*t]
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("`{t}` is not a table"))?;
    }
    table[*last] = toml_edit::value(parse_value(raw));
    let out = doc.to_string();
    // Validate: a string value for a typed key (e.g. mode = "full") is fine,
    // an unknown key or invalid enum value is not.
    let parsed =
        Config::parse(&out).map_err(|e| anyhow::anyhow!("invalid value for `{key}`: {e}"))?;
    get(&parsed, key)?;
    Ok(out)
}

/// Set a key in the user config file, creating the file if needed.
pub fn set_in_file(file: &Path, key: &str, raw: &str) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let updated = set_in_text(&text, key, raw)?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, updated)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_CONFIG_TOML;
    use crate::permissions::Mode;

    #[test]
    fn get_reads_effective_values() {
        let c = Config::default();
        assert_eq!(get(&c, "permissions.mode").unwrap(), "workspace");
        assert_eq!(get(&c, "inference.provider").unwrap(), "local");
        assert_eq!(get(&c, "agent.max_recovery_attempts").unwrap(), "8");
        assert!(get(&c, "permissions.nope").is_err());
    }

    #[test]
    fn set_preserves_comments_and_validates() {
        let out = set_in_text(DEFAULT_CONFIG_TOML, "permissions.mode", "ask").unwrap();
        assert!(out.contains("# Kara configuration"));
        assert_eq!(Config::parse(&out).unwrap().permissions.mode, Mode::Ask);

        let out = set_in_text(&out, "agent.max_recovery_attempts", "3").unwrap();
        assert_eq!(Config::parse(&out).unwrap().agent.max_recovery_attempts, 3);

        let out = set_in_text(&out, "inference.endpoint", "http://10.0.0.2:7878/v1").unwrap();
        assert_eq!(
            Config::parse(&out).unwrap().inference.endpoint,
            "http://10.0.0.2:7878/v1"
        );

        let out = set_in_text(&out, "inference.local.gpu_layers", "0").unwrap();
        assert_eq!(Config::parse(&out).unwrap().inference.local.gpu_layers, 0);

        assert!(set_in_text(&out, "permissions.mode", "everything").is_err());
        assert!(set_in_text(&out, "permissions.unknown", "1").is_err());
        assert!(set_in_text(&out, "agent.max_recovery_attempts", "many").is_err());
        assert!(set_in_text(&out, "mode", "ask").is_err());
    }

    #[test]
    fn set_creates_missing_file_and_tables() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sub/config.toml");
        set_in_file(&f, "permissions.mode", "full").unwrap();
        let c = Config::load_user(&f).unwrap();
        assert_eq!(c.permissions.mode, Mode::Full);
    }
}
