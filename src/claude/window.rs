//! Context-window table + threshold selection (PLAN §5.2).

use crate::config::Config;

/// Which of the three `ctx_*` tokens a percentage maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtxLevel {
    Ok,
    Warn,
    Hot,
}

impl CtxLevel {
    /// The herdr token key (`ctx_ok` / `ctx_warn` / `ctx_hot`).
    pub fn token(self) -> &'static str {
        match self {
            CtxLevel::Ok => "ctx_ok",
            CtxLevel::Warn => "ctx_warn",
            CtxLevel::Hot => "ctx_hot",
        }
    }

    /// All three keys, for clearing the two that are not active.
    pub const ALL: [&'static str; 3] = ["ctx_ok", "ctx_warn", "ctx_hot"];
}

/// Resolve the context window for a model + observed usage (PLAN §5.2):
///   1. An explicit `[context_window.by_model]` override always wins.
///   2. Otherwise the default (200k), promoted to 1M when `auto_promote_1m` is on
///      and observed usage already exceeds the default (a 200k window is then
///      impossible).
pub fn resolve_window(model_id: &str, used: u64, cfg: &Config) -> u64 {
    if let Some(&w) = cfg.by_model.get(model_id) {
        return w;
    }
    let default = cfg.ctx_default_window;
    if cfg.auto_promote_1m && used > default {
        return 1_000_000;
    }
    default
}

/// Percentage of the window used, rounded and clamped to 0..=100.
pub fn pct(used: u64, window: u64) -> u8 {
    if window == 0 {
        return 100;
    }
    let raw = (100.0 * used as f64 / window as f64).round();
    raw.clamp(0.0, 100.0) as u8
}

/// Select the token level for a percentage given warn/hot thresholds.
pub fn select(pct: u8, warn: u8, hot: u8) -> CtxLevel {
    if pct < warn {
        CtxLevel::Ok
    } else if pct < hot {
        CtxLevel::Warn
    } else {
        CtxLevel::Hot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn pct_rounds_and_clamps() {
        assert_eq!(pct(0, 200_000), 0);
        assert_eq!(pct(24_000, 200_000), 12);
        assert_eq!(pct(88_000, 200_000), 44);
        assert_eq!(pct(182_000, 200_000), 91);
        assert_eq!(pct(200_000, 200_000), 100);
        assert_eq!(pct(500_000, 200_000), 100); // clamp
                                                // round-half: 1000/200000 = 0.5% → rounds to 1 (round-half-away).
        assert_eq!(pct(1_000, 200_000), 1);
    }

    #[test]
    fn threshold_selection_defaults() {
        assert_eq!(select(12, 50, 80), CtxLevel::Ok);
        assert_eq!(select(44, 50, 80), CtxLevel::Ok);
        assert_eq!(select(49, 50, 80), CtxLevel::Ok);
        assert_eq!(select(50, 50, 80), CtxLevel::Warn);
        assert_eq!(select(79, 50, 80), CtxLevel::Warn);
        assert_eq!(select(80, 50, 80), CtxLevel::Hot);
        assert_eq!(select(91, 50, 80), CtxLevel::Hot);
    }

    #[test]
    fn window_default_and_override() {
        let mut c = cfg();
        assert_eq!(resolve_window("claude-opus-4-8", 100_000, &c), 200_000);
        c.by_model.insert("claude-opus-4-8".to_string(), 1_000_000);
        assert_eq!(resolve_window("claude-opus-4-8", 100_000, &c), 1_000_000);
    }

    #[test]
    fn auto_promote_1m_when_over_default() {
        let c = cfg(); // auto_promote_1m = true
                       // Under 200k → stays 200k.
        assert_eq!(resolve_window("claude-opus-4-8", 150_000, &c), 200_000);
        // Over 200k → promoted to 1M.
        assert_eq!(resolve_window("claude-opus-4-8", 250_000, &c), 1_000_000);
        // 250k / 1M → 25%, not clamped to 100.
        let w = resolve_window("claude-opus-4-8", 250_000, &c);
        assert_eq!(pct(250_000, w), 25);
    }

    #[test]
    fn auto_promote_disabled_clamps_instead() {
        let mut c = cfg();
        c.auto_promote_1m = false;
        assert_eq!(resolve_window("claude-opus-4-8", 250_000, &c), 200_000);
        assert_eq!(pct(250_000, 200_000), 100);
    }
}
