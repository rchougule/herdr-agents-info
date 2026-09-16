//! `RowTokens` → `ReportPlan` (§5.2 rule 2 of `docs/naming-framing.md`).
//!
//! Full reports only: every report a pane receives sets or clears **all 9**
//! owned tokens (`$ctx_ok/warn/hot $model $disk $tab $d2 $pane $d3`,
//! `docs/layout-design.md` §3.1). There is no partial "name-only" report and
//! no optional "skip this field" state — a subset report is a bug.
//!
//! Every report also clears the **retired** class-per-line keys
//! (`t1 t2 t3 d1 mo1 mo2 mo3`) unconditionally, belt-and-braces, so a user
//! still running the old `rows_by_agent.claude` config block never renders
//! stale text from before the layout redesign — clearing a token herdr's
//! current config does not reference is a no-op.
//!
//! `login`/`org` are appended after the owned set only when the config opts
//! in (default off; identical on every row otherwise).

use serde::Serialize;

use crate::pack::RowTokens;

/// The exact token patch for one pane. `Serialize` backs the full-fleet insta
/// snapshot (`tests/snapshot.rs`, §11 fixture 23 of naming-framing.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportPlan {
    pub pane_id: String,
    /// Ordered `(key, value)` tokens to set.
    pub set: Vec<(String, String)>,
    /// Token keys to clear.
    pub clear: Vec<String>,
    pub seq: u64,
}

/// The 9 owned token keys (layout-design §3.1), in the fixed order every
/// report emits them: line 1 (`ctx_*`, `model`, `disk`), then line 2 (`tab`,
/// `d2`), then line 3 (`pane`, `d3`). Each is either set (non-empty value) or
/// cleared (empty value) — never omitted.
fn owned(tokens: &RowTokens) -> [(&'static str, &str); 9] {
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
    ]
}

/// The retired class-per-line keys from the old (pre layout-design) packer.
/// Always cleared, never set — belt-and-braces against a stale config block.
const RETIRED: [&str; 7] = ["t1", "t2", "t3", "d1", "mo1", "mo2", "mo3"];

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
    for key in RETIRED {
        clear.push(key.to_string());
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
        // 9 owned keys + 7 retired keys, every one set-or-cleared.
        let p = report_plan("w1:p1", &tokens(), 7, None);
        let mut keys: Vec<&str> = p
            .set
            .iter()
            .map(|(k, _)| k.as_str())
            .chain(p.clear.iter().map(|k| k.as_str()))
            .collect();
        keys.sort();
        let mut expect = vec![
            "ctx_ok", "ctx_warn", "ctx_hot", "model", "disk", "tab", "d2", "pane", "d3", "t1",
            "t2", "t3", "d1", "mo1", "mo2", "mo3",
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
            vec![
                "ctx_warn", "ctx_hot", "disk", "d2", "pane", "d3", "t1", "t2", "t3", "d1", "mo1",
                "mo2", "mo3",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
        assert_eq!(p.seq, 7);
    }

    #[test]
    fn retired_keys_are_always_cleared_never_set() {
        // Belt-and-braces: even a fully-populated row clears the old keys.
        let full = RowTokens {
            ctx_ok: "10%".into(),
            model: "opus".into(),
            tab: "t".into(),
            d2: "d".into(),
            pane: "".into(),
            d3: "".into(),
            ..RowTokens::default()
        };
        let p = report_plan("w1:p1", &full, 1, None);
        for k in ["t1", "t2", "t3", "d1", "mo1", "mo2", "mo3"] {
            assert!(p.clear.contains(&k.to_string()), "{k} should be cleared");
            assert!(
                !p.set.iter().any(|(sk, _)| sk == k),
                "{k} should never be set"
            );
        }
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
