//! Plugin configuration (PLAN Appendix B). Read from
//! `$HERDR_PLUGIN_CONFIG_DIR/config.toml`; every field defaults so a missing or
//! partial file is fine.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

/// Login token mode. `"auto"` is reserved for a future release and treated as `Off` for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoginMode {
    #[default]
    Off,
    Always,
}

impl LoginMode {
    fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "always" => LoginMode::Always,
            // "off", "auto" (P2, not yet triggerable), and anything else → off.
            _ => LoginMode::Off,
        }
    }
    pub fn is_on(self) -> bool {
        matches!(self, LoginMode::Always)
    }
}

/// Assumed-width layout knobs for the class-per-line packer (`src/pack.rs`, §7).
///
/// herdr never tells the plugin the live sidebar width, so the packer works off
/// an *assumed* width (default 26). From it two usable budgets are derived: line
/// 1 loses the 1-col divider + the icon+space (`assumed_width - 2`), later lines
/// lose the divider + the 3-col indent (`assumed_width - 4`). Both derivations
/// are overridable so a user on a wider or narrower sidebar can retune without a
/// rebuild.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Assumed sidebar width in columns (the single knob most users set).
    pub assumed_width: usize,
    /// Usable columns on line 1 (default `assumed_width - 2`).
    pub line1_usable: usize,
    /// Usable columns on lines 2+ (default `assumed_width - 4`).
    pub other_usable: usize,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            assumed_width: 26,
            line1_usable: 24,
            other_usable: 22,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub login: LoginMode,
    pub show_model: bool,
    pub warn: u8,
    pub hot: u8,
    pub ctx_default_window: u64,
    pub auto_promote_1m: bool,
    pub by_model: HashMap<String, u64>,
    pub layout: LayoutConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            login: LoginMode::Off,
            show_model: true,
            warn: 50,
            hot: 80,
            ctx_default_window: 200_000,
            auto_promote_1m: true,
            by_model: HashMap::new(),
            layout: LayoutConfig::default(),
        }
    }
}

/// Raw TOML mirror; all optional so a partial file still parses.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    tokens: RawTokens,
    #[serde(default)]
    thresholds: RawThresholds,
    #[serde(default)]
    context_window: RawContextWindow,
    #[serde(default)]
    layout: RawLayout,
}

#[derive(Debug, Default, Deserialize)]
struct RawLayout {
    assumed_width: Option<usize>,
    line1_usable: Option<usize>,
    other_usable: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTokens {
    login: Option<String>,
    model: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawThresholds {
    warn: Option<u8>,
    hot: Option<u8>,
}

#[derive(Debug, Default, Deserialize)]
struct RawContextWindow {
    default: Option<u64>,
    auto_promote_1m: Option<bool>,
    #[serde(default)]
    by_model: HashMap<String, u64>,
}

impl Config {
    /// Parse from a TOML string, filling defaults for anything absent.
    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        let raw: RawConfig = toml::from_str(s)?;
        let d = Config::default();
        Ok(Config {
            login: raw
                .tokens
                .login
                .as_deref()
                .map(LoginMode::from_str)
                .unwrap_or(d.login),
            show_model: raw.tokens.model.unwrap_or(d.show_model),
            warn: raw.thresholds.warn.unwrap_or(d.warn),
            hot: raw.thresholds.hot.unwrap_or(d.hot),
            ctx_default_window: raw.context_window.default.unwrap_or(d.ctx_default_window),
            auto_promote_1m: raw
                .context_window
                .auto_promote_1m
                .unwrap_or(d.auto_promote_1m),
            by_model: raw.context_window.by_model,
            layout: {
                let assumed = raw.layout.assumed_width.unwrap_or(d.layout.assumed_width);
                LayoutConfig {
                    assumed_width: assumed,
                    line1_usable: raw
                        .layout
                        .line1_usable
                        .unwrap_or_else(|| assumed.saturating_sub(2)),
                    other_usable: raw
                        .layout
                        .other_usable
                        .unwrap_or_else(|| assumed.saturating_sub(4)),
                }
            },
        })
    }

    /// Load from `$HERDR_PLUGIN_CONFIG_DIR/config.toml`. A missing dir or file
    /// yields defaults; a malformed file falls back to defaults (a broken config
    /// must never take the sidebar down).
    pub fn load() -> Self {
        let Some(dir) = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR") else {
            return Config::default();
        };
        let path = PathBuf::from(dir).join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => Config::from_toml_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_is_all_defaults() {
        let c = Config::from_toml_str("").unwrap();
        assert_eq!(c.login, LoginMode::Off);
        assert!(c.show_model);
        assert_eq!(c.warn, 50);
        assert_eq!(c.hot, 80);
        assert_eq!(c.ctx_default_window, 200_000);
        assert!(c.auto_promote_1m);
        assert!(c.by_model.is_empty());
    }

    #[test]
    fn appendix_b_recipe_parses() {
        let s = r#"
[tokens]
login = "off"
model = true

[thresholds]
warn = 50
hot  = 80

[context_window]
default = 200000
auto_promote_1m = true
[context_window.by_model]
"claude-opus-4-8" = 1000000
"#;
        let c = Config::from_toml_str(s).unwrap();
        assert_eq!(c.login, LoginMode::Off);
        assert_eq!(c.by_model.get("claude-opus-4-8"), Some(&1_000_000));
    }

    #[test]
    fn login_always_and_auto() {
        assert_eq!(LoginMode::from_str("always"), LoginMode::Always);
        assert_eq!(LoginMode::from_str("auto"), LoginMode::Off); // P2, not yet
        assert_eq!(LoginMode::from_str("off"), LoginMode::Off);
        assert!(LoginMode::Always.is_on());
        assert!(!LoginMode::Off.is_on());
    }

    #[test]
    fn partial_overrides_keep_other_defaults() {
        let c = Config::from_toml_str("[thresholds]\nwarn = 40\n").unwrap();
        assert_eq!(c.warn, 40);
        assert_eq!(c.hot, 80); // untouched default
    }

    #[test]
    fn layout_defaults() {
        let c = Config::from_toml_str("").unwrap();
        assert_eq!(c.layout.assumed_width, 26);
        assert_eq!(c.layout.line1_usable, 24);
        assert_eq!(c.layout.other_usable, 22);
    }

    #[test]
    fn layout_assumed_width_rederives_usable_budgets() {
        let c = Config::from_toml_str("[layout]\nassumed_width = 40\n").unwrap();
        assert_eq!(c.layout.assumed_width, 40);
        assert_eq!(c.layout.line1_usable, 38); // 40 - 2
        assert_eq!(c.layout.other_usable, 36); // 40 - 4
    }

    #[test]
    fn layout_explicit_usable_overrides_win_over_derivation() {
        let c = Config::from_toml_str(
            "[layout]\nassumed_width = 40\nline1_usable = 30\nother_usable = 28\n",
        )
        .unwrap();
        assert_eq!(c.layout.line1_usable, 30);
        assert_eq!(c.layout.other_usable, 28);
    }
}
