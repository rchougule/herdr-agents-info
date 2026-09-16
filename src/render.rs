//! `RowTokens` → `ReportPlan` (`docs/DESIGN.md`, Architecture).
//!
//! Full reports only: every report a pane receives sets or clears **all 10**
//! owned tokens (`$ctx_ok/warn/hot $model $disk $tab $d2 $pane $d3 $sep`,
//! `docs/DESIGN.md` Placement). There is no partial "name-only" report and
//! no optional "skip this field" state — a subset report is a bug.
//!
//! `login`/`org` are appended after the owned set only when the config opts
//! in (default off; identical on every row otherwise). herdr caps a report at
//! 16 tokens, so the owned set (10) plus `login`+`org` (2) stays within budget.

use serde::Serialize;

use crate::pack::RowTokens;

/// The exact token patch for one pane. `Serialize` backs the full-fleet insta
/// snapshot (`tests/snapshot.rs`, see `tests/snapshot.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportPlan {
    pub pane_id: String,
    /// Ordered `(key, value)` tokens to set.
    pub set: Vec<(String, String)>,
    /// Token keys to clear.
    pub clear: Vec<String>,
    pub seq: u64,
}

/// The 10 owned token keys (`docs/DESIGN.md` Placement), in the fixed order every
/// report emits them: `ctx_*`, `model`, `disk` (metadata line), `tab`, `d2`,
/// `pane`, `d3` (identity lines), and `sep` (the between-entry rule). Each is
/// either set (non-empty value) or cleared (empty value) — never omitted. herdr
/// caps a report at 16 tokens, which this (10, +2 optional login/org) fits.
fn owned(tokens: &RowTokens) -> [(&'static str, &str); 10] {
    [
        ("ctx_ok", tokens.ctx_ok.as_str()),
        ("ctx_warn", tokens.ctx_warn.as_str()),
        ("ctx_hot", tokens.ctx_hot.as_str()),
        ("model", tokens.model.as_str()),
        ("disk", tokens.disk.as_str()),
        ("tab", tokens.tab.as_str()),
        ("d2", tokens.d2.as_str()),
        ("pane", tokens.pane.as_str()),
        ("d3", tokens.d3.as_str()),
        ("sep", tokens.sep.as_str()),
    ]
}

/// Build the full set-or-clear report for a pane from its computed tokens.
/// `login` is `Some((email, org))` only when `[tokens] login = "always"`; either
/// string may be empty (→ cleared).
pub fn report_plan(
    pane_id: &str,
    tokens: &RowTokens,
    seq: u64,
    login: Option<(&str, &str)>,
) -> ReportPlan {
    let mut set = Vec::new();
    let mut clear = Vec::new();
    for (key, value) in owned(tokens) {
        if value.is_empty() {
            clear.push(key.to_string());
        } else {
            set.push((key.to_string(), value.to_string()));
        }
    }
    if let Some((email, org)) = login {
        for (key, value) in [("login", email), ("org", org)] {
            if value.is_empty() {
                clear.push(key.to_string());
            } else {
                set.push((key.to_string(), value.to_string()));
            }
        }
    }
    ReportPlan {
        pane_id: pane_id.to_string(),
        set,
        clear,
        seq,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> RowTokens {
        RowTokens {
            tab: "flow".into(),
            model: "opus".into(),
            ctx_ok: "44%".into(),
            ..RowTokens::default()
        }
    }

    #[test]
    fn report_is_always_full() {
        // All 10 owned keys, every one set-or-cleared, within herdr's 16-token cap.
        let p = report_plan("w1:p1", &tokens(), 7, None);
        assert!(
            p.set.len() + p.clear.len() <= 16,
            "must fit herdr's 16-token cap"
        );
        let mut keys: Vec<&str> = p
            .set
            .iter()
            .map(|(k, _)| k.as_str())
            .chain(p.clear.iter().map(|k| k.as_str()))
            .collect();
        keys.sort();
        let mut expect = vec![
            "ctx_ok", "ctx_warn", "ctx_hot", "model", "disk", "tab", "d2", "pane", "d3", "sep",
        ];
        expect.sort();
        assert_eq!(keys, expect);
    }

    #[test]
    fn set_clear_split_and_order() {
        let p = report_plan("w1:p1", &tokens(), 7, None);
        assert_eq!(
            p.set,
            vec![
                ("ctx_ok".into(), "44%".into()),
                ("model".into(), "opus".into()),
                ("tab".into(), "flow".into()),
            ]
        );
        assert_eq!(
            p.clear,
            vec!["ctx_warn", "ctx_hot", "disk", "d2", "pane", "d3", "sep"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(p.seq, 7);
    }

    #[test]
    fn login_appended_only_when_present() {
        let p = report_plan("w1:p1", &tokens(), 7, Some(("me@x", "Org")));
        assert!(p.set.contains(&("login".into(), "me@x".into())));
        assert!(p.set.contains(&("org".into(), "Org".into())));
        // Off by default: no login/org keys at all.
        let p = report_plan("w1:p1", &tokens(), 7, None);
        assert!(!p.set.iter().any(|(k, _)| k == "login" || k == "org"));
        assert!(!p.clear.iter().any(|k| k == "login" || k == "org"));
    }
}
