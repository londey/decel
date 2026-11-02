// SPDX-License-Identifier: MIT OR Apache-2.0

//! `config.toml` (DESIGN §8.7): keymap overrides, theme, font, peek key,
//! column headers and engine limits. Hot-reloaded by polling its mtime;
//! invalid files are reported and otherwise ignored.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnHeaders {
    #[default]
    Numbers,
    Letters,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: ThemeChoice,
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    pub peek_key: String,
    pub column_headers: ColumnHeaders,
    pub instruction_budget: u64,
    pub memory_limit_mb: u64,
    /// Action name → keystrokes, e.g. `fill_down = "ctrl-shift-j"`.
    pub keymap: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            font_family: None,
            font_size: None,
            peek_key: "`".into(),
            column_headers: ColumnHeaders::Numbers,
            instruction_budget: 10_000_000,
            memory_limit_mb: 512,
            keymap: BTreeMap::new(),
        }
    }
}

pub const TEMPLATE: &str = r#"# decel configuration. Changes apply while decel is running.

# theme = "system"          # "system", "light" or "dark"
# font_family = "Inter"
# font_size = 14
# peek_key = "`"            # hold to colour cells by metatable
# column_headers = "numbers" # or "letters" (display only)
# instruction_budget = 10000000  # per hook call (applies to workbooks opened afterwards)
# memory_limit_mb = 512

[keymap]
# fill_down = "ctrl-shift-j"
"#;

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        if c.peek_key.is_empty() {
            return Err("peek_key must not be empty".into());
        }
        if c.instruction_budget < 1000 {
            return Err("instruction_budget must be at least 1000".into());
        }
        if !(16..=65536).contains(&c.memory_limit_mb) {
            return Err("memory_limit_mb must be between 16 and 65536".into());
        }
        if let Some(s) = c.font_size
            && !(6.0..=72.0).contains(&s)
        {
            return Err("font_size must be between 6 and 72".into());
        }
        Ok(c)
    }

    pub fn limits(&self) -> decel_engine::Limits {
        decel_engine::Limits {
            instructions: self.instruction_budget,
            memory: (self.memory_limit_mb as usize).saturating_mul(1024 * 1024),
            ..decel_engine::Limits::default()
        }
    }
}

pub fn path() -> Option<PathBuf> {
    crate::recent::config_dir().map(|d| d.join("config.toml"))
}

/// Loads the config. Missing file: defaults. Invalid file: defaults plus an
/// error message.
pub fn load() -> (Config, Option<String>) {
    let Some(p) = path() else {
        return (Config::default(), None);
    };
    match std::fs::read_to_string(&p) {
        Ok(text) => match Config::parse(&text) {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("{}: {e}", p.display()))),
        },
        Err(_) => (Config::default(), None),
    }
}

/// Creates the config file from the template if it doesn't exist.
pub fn ensure_exists() -> Option<PathBuf> {
    let p = path()?;
    if !p.exists() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, TEMPLATE);
    }
    Some(p)
}

pub fn modified() -> Option<SystemTime> {
    std::fs::metadata(path()?).and_then(|m| m.modified()).ok()
}

pub const POLL: Duration = Duration::from_secs(1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_and_empty_parse_to_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::parse(TEMPLATE).unwrap(), Config::default());
    }

    #[test]
    fn parses_everything() {
        let c = Config::parse(
            r#"
            theme = "dark"
            font_family = "Inter"
            font_size = 15
            peek_key = "f12"
            column_headers = "letters"
            instruction_budget = 5000000
            memory_limit_mb = 256
            [keymap]
            fill_down = "ctrl-shift-j"
            "#,
        )
        .unwrap();
        assert_eq!(c.theme, ThemeChoice::Dark);
        assert_eq!(c.column_headers, ColumnHeaders::Letters);
        assert_eq!(c.keymap["fill_down"], "ctrl-shift-j");
        assert_eq!(c.limits().memory, 256 * 1024 * 1024);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::parse("theme = \"purple\"").is_err());
        assert!(Config::parse("unknown_key = 1").is_err());
        assert!(Config::parse("memory_limit_mb = 1").is_err());
        assert!(Config::parse("peek_key = \"\"").is_err());
        assert!(Config::parse("font_size = 500").is_err());
    }
}
