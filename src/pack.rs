//! Fixed field→slot packing (`docs/DESIGN.md`, Placement). Every field has
//! exactly one home; there is no packer float.
//!
//! Nine owned tokens, every one set-or-cleared on every report:
//!
//! ```text
//! line 1: state_icon · workspace(bold)
//! line 2:             $tab(normal) · $d2(dim)
//! line 3:             $pane(normal) · $d3(dim)
//! line 4:             $disk · $model(dim) · $ctx_ok|$ctx_warn|$ctx_hot(color)
//! ```
//!
//! The **workspace leader has line 1 to itself** (herdr's own bold token), so it
//! is never crowded or truncated by metadata. The metadata (`disk · model ·
//! ctx%`) lives on line 4; each field carries an optional configured icon.
//! `$disk` is only ever set above the configured threshold, so it appears
//! rarely, as an alert, and herdr drops it when empty.
//!
//! `tab` never shares a line with `pane`; a derived item (splitter §4 / hint §6)
//! sits after the pane when there is one, else after the tab (`$d3` vs `$d2`) —
//! never both. herdr draws ` · ` only between visible tokens and drops empty
//! lines, so empty tokens cost nothing.

use serde::{Deserialize, Serialize};

use crate::claude::window::{self, CtxLevel};
use crate::config::{IconConfig, LayoutConfig};

/// Prepend a configured icon (`"💾"`) to a value (`"9.6G"` → `"💾 9.6G"`), or
/// return the value unchanged when no icon is set for that field.
fn with_icon(icon: &Option<String>, value: &str) -> String {
    match icon {
        Some(i) => format!("{i} {value}"),
        None => value.to_string(),
    }
}

/// The separator herdr renders between visible tokens (` · `, 3 columns), and
/// the one the packer joins multiple derived items with. Mirrored here so the
/// packer's budgeting matches the render.
const SEP: &str = " · ";
const SEP_W: usize = 3;

/// The 9 owned tokens for one Claude row (`docs/DESIGN.md` Placement). Empty means
/// "cleared". This is what `render::report_plan` turns into a full
/// set-or-clear report (plus the retired-key clears) and what the per-pane
/// cache compares for the idempotent skip (`docs/DESIGN.md`, Architecture).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowTokens {
    pub ctx_ok: String,
    pub ctx_warn: String,
    pub ctx_hot: String,
    pub model: String,
    pub disk: String,
    pub tab: String,
    pub d2: String,
    pub pane: String,
    pub d3: String,
}

fn width(s: &str) -> usize {
    s.chars().count()
}

/// Tail-truncate to `budget` columns with a trailing `…` (1 column). The last
/// resort — only for an item that does not fit its line whole.
fn truncate(s: &str, budget: usize) -> String {
    if width(s) <= budget {
        return s.to_string();
    }
    if budget == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(budget - 1).collect();
    out.push('…');
    out
}

/// Fit one line's identity item (`tab` or `pane`) alongside its derived item
/// (`d2`/`d3`), keeping the derived item WHOLE and tail-cutting the identity
/// item to make room (`docs/DESIGN.md` Placement: "the one place identity is cut for
/// something other than the line edge"). When there is no derived item, the
/// identity item is simply truncated to the line budget (the honest tail-cut,
/// unchanged from before). When the identity item is absent (a thin row's
/// hint has the line to itself), the derived item takes the whole budget.
fn fit_with_derived(identity: &str, derived: &str, budget: usize) -> (String, String) {
    if derived.is_empty() {
        return (truncate(identity, budget), String::new());
    }
    if identity.is_empty() {
        return (String::new(), truncate(derived, budget));
    }
    let need = width(identity) + SEP_W + width(derived);
    if need <= budget {
        return (identity.to_string(), derived.to_string());
    }
    let avail = budget.saturating_sub(SEP_W + width(derived));
    (truncate(identity, avail), derived.to_string())
}

/// Pack a pane's fields into the 9 fixed tokens (`docs/DESIGN.md` Placement).
/// The workspace leader is herdr's own built-in token on line 1 and is not
/// packed here; the plugin only fills the tokens below.
///
/// - `tab` — rung 1 (§3), when shown.
/// - `pane` — rung 2 (`agent_name ?? pane_label`), when shown.
/// - `derived` — splitters (§4) / hint (§6), in order; joined by ` · ` and
///   assigned to `d3` when a pane is present, else `d2`.
/// - `model` — the short model form (`opus`), when known and enabled. On the
///   metadata line (`disk · model · ctx%`), with its optional icon.
/// - `pct` — context percentage; when present, colours `$ctx_ok`/`warn`/`hot`
///   by threshold.
#[allow(clippy::too_many_arguments)]
pub fn pack(
    tab: Option<&str>,
    pane: Option<&str>,
    derived: &[&str],
    model: Option<&str>,
    pct: Option<u8>,
    disk: Option<&str>,
    layout: &LayoutConfig,
    icons: &IconConfig,
    warn: u8,
    hot: u8,
) -> RowTokens {
    let mut rt = RowTokens::default();

    // Metadata line (`disk · model · ctx%`): all short, on their own line, so
    // none is truncated here — the workspace has line 1 to itself and is never
    // cut for them. Each carries its configured icon when set.

    // $ctx_* — one of three, by threshold. Never moved.
    if let Some(p) = pct {
        let value = with_icon(&icons.context, &format!("{p}%"));
        match window::select(p, warn, hot) {
            CtxLevel::Ok => rt.ctx_ok = value,
            CtxLevel::Warn => rt.ctx_warn = value,
            CtxLevel::Hot => rt.ctx_hot = value,
        }
    }

    // $model — the short model form, set as-is (model_short already caps it).
    if let Some(m) = model {
        rt.model = with_icon(&icons.model, m);
    }

    // $disk — pre-gated and pre-formatted by the caller (only ever present above
    // the threshold, so the line appears rarely, as an alert).
    if let Some(d) = disk {
        rt.disk = with_icon(&icons.disk, d);
    }

    // $tab / $pane / $d2 / $d3 — the derived item follows the pane when one
    // is present, else the tab (`docs/DESIGN.md` Placement assignment rule).
    let derived_joined = derived.join(SEP);
    let budget = layout.other_usable;

    if let Some(p) = pane {
        rt.tab = truncate(tab.unwrap_or(""), budget);
        let (pane_out, d3_out) = fit_with_derived(p, &derived_joined, budget);
        rt.pane = pane_out;
        rt.d3 = d3_out;
    } else {
        let (tab_out, d2_out) = fit_with_derived(tab.unwrap_or(""), &derived_joined, budget);
        rt.tab = tab_out;
        rt.d2 = d2_out;
    }

    rt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> LayoutConfig {
        LayoutConfig::default() // line1_usable 24, other_usable 22
    }

    fn no_icons() -> IconConfig {
        IconConfig::default()
    }

    // `_ws` is kept for readable call sites; the workspace is herdr's own token
    // and no longer an input to the packer.
    fn pk(
        _ws: &str,
        tab: Option<&str>,
        pane: Option<&str>,
        derived: &[&str],
        model: Option<&str>,
        pct: Option<u8>,
    ) -> RowTokens {
        pack(
            tab,
            pane,
            derived,
            model,
            pct,
            None,
            &layout(),
            &no_icons(),
            50,
            80,
        )
    }

    fn pk_disk(_ws: &str, model: Option<&str>, pct: Option<u8>, disk: Option<&str>) -> RowTokens {
        pack(
            None,
            None,
            &[],
            model,
            pct,
            disk,
            &layout(),
            &no_icons(),
            50,
            80,
        )
    }

    // ── Basic assignment ───────────────────────────────────────────────────

    #[test]
    fn empty_row_is_all_empty() {
        assert_eq!(
            pk("solo", None, None, &[], None, None),
            RowTokens::default()
        );
    }

    #[test]
    fn no_metadata_clears_model_and_ctx() {
        let rt = pk("solo", Some("feat"), None, &[], None, None);
        assert_eq!(rt.tab, "feat");
        assert_eq!(rt.model, "");
        assert_eq!(rt.ctx_ok, "");
        assert_eq!(rt.ctx_warn, "");
        assert_eq!(rt.ctx_hot, "");
    }

    #[test]
    fn ctx_bands_select_the_right_token() {
        assert_eq!(pk("w", None, None, &[], None, Some(12)).ctx_ok, "12%");
        assert_eq!(pk("w", None, None, &[], None, Some(55)).ctx_warn, "55%");
        assert_eq!(pk("w", None, None, &[], None, Some(91)).ctx_hot, "91%");
    }

    #[test]
    fn honest_tail_cut_when_item_exceeds_every_line() {
        // lci fast / tab "remaining lci elephants" (23 > 22) / no transcript.
        let rt = pk(
            "lci fast",
            Some("remaining lci elephants"),
            None,
            &[],
            None,
            None,
        );
        assert_eq!(width(&rt.tab), 22);
        assert!(rt.tab.ends_with('…'));
        assert_eq!(rt.pane, "");
        assert_eq!(rt.model, "");
    }

    // ── Fixture 21 replacement + the layout additions ──────────

    #[test]
    fn each_field_has_one_token() {
        // The richest combination — tab, pane, model, ctx and a derived
        // splitter all present at once — shows every one of the 8 tokens
        // carries exactly its own field's content, never another's.
        let rt = pk(
            "experiment",
            Some("2"),
            Some("plugin"),
            &["p4V"],
            Some("opus"),
            Some(5),
        );
        assert_eq!(rt.tab, "2");
        assert_eq!(rt.pane, "plugin");
        assert_eq!(rt.model, "opus");
        assert_eq!(rt.ctx_ok, "5%");
        assert_eq!(rt.ctx_warn, "");
        assert_eq!(rt.ctx_hot, "");
        // Pane is present, so the splitter follows it into d3; d2 stays empty.
        assert_eq!(rt.d3, "p4V");
        assert_eq!(rt.d2, "");
        // No cross-contamination between fields.
        assert!(!rt.tab.contains("plugin") && !rt.tab.contains("opus") && !rt.tab.contains("p4V"));
        assert!(!rt.pane.contains("opus") && !rt.pane.contains("p4V") && !rt.pane.contains('2'));
        assert!(!rt.model.contains("plugin") && !rt.model.contains("p4V"));
        // At most one of d2/d3 is ever non-empty.
        assert!(rt.d2.is_empty() || rt.d3.is_empty());
    }

    #[test]
    fn model_never_on_lines_2_3() {
        // Even under width pressure that clears/cuts the model, it can only
        // ever land in the `model` field — never `tab`, `pane`, `d2` or `d3`
        // (the old float, and the bug it caused, are both gone).
        let rt = pk(
            "a-very-long-workspace",
            Some("t"),
            None,
            &["hint"],
            Some("opus"),
            Some(5),
        );
        assert!(!rt.tab.contains("opus"));
        assert!(!rt.d2.contains("opus"));
        assert_eq!(rt.pane, "");
        assert_eq!(rt.d3, "");
    }

    #[test]
    fn tab_never_on_line_1() {
        // A long tab never spills into the line-1 fields (model/ctx); it is
        // tail-cut on its own line instead.
        let rt = pk(
            "w",
            Some("some very long tab name that exceeds the budget"),
            None,
            &[],
            Some("opus"),
            Some(5),
        );
        assert_eq!(rt.model, "opus");
        assert_eq!(rt.ctx_ok, "5%");
        assert!(width(&rt.tab) <= layout().other_usable);
    }

    #[test]
    fn pane_never_on_line_2() {
        // tab and pane never share a field, regardless of which are present.
        let rt = pk("w", Some("tabtext"), Some("panetext"), &[], None, None);
        assert_eq!(rt.tab, "tabtext");
        assert_eq!(rt.pane, "panetext");
        assert!(!rt.tab.contains("panetext"));
        assert!(!rt.pane.contains("tabtext"));
    }

    #[test]
    fn derived_follows_pane_when_present() {
        let with_pane = pk("w", Some("2"), Some("plugin"), &["p4V"], None, None);
        assert_eq!(with_pane.d3, "p4V");
        assert_eq!(with_pane.d2, "");

        let without_pane = pk("w", Some("2"), None, &["p4V"], None, None);
        assert_eq!(without_pane.d2, "p4V");
        assert_eq!(without_pane.d3, "");
    }

    #[test]
    fn model_unaffected_by_workspace_length() {
        // The workspace has line 1 to itself; the model lives on the metadata
        // line, so a long workspace never cuts or clears it.
        let rt = pk(
            "a-very-long-workspace-name",
            None,
            None,
            &[],
            Some("opus"),
            Some(42),
        );
        assert_eq!(rt.ctx_ok, "42%");
        assert_eq!(rt.model, "opus");
    }

    #[test]
    fn icons_prepend_on_the_metadata_fields() {
        let icons = IconConfig {
            disk: Some("D".into()),
            model: Some("M".into()),
            context: Some("C".into()),
        };
        let rt = pack(
            None,
            None,
            &[],
            Some("opus"),
            Some(42),
            Some("9.6G"),
            &layout(),
            &icons,
            50,
            80,
        );
        assert_eq!(rt.model, "M opus");
        assert_eq!(rt.ctx_ok, "C 42%");
        assert_eq!(rt.disk, "D 9.6G");
    }

    // ── $disk (its own line) ───────────────────────────────────────────────

    #[test]
    fn disk_set_on_its_own_token() {
        let rt = pk_disk("study", Some("opus"), Some(12), Some("1.2G"));
        assert_eq!(rt.model, "opus");
        assert_eq!(rt.ctx_ok, "12%");
        assert_eq!(rt.disk, "1.2G");
    }

    #[test]
    fn disk_absent_when_not_provided() {
        let rt = pk_disk("study", Some("opus"), Some(12), None);
        assert_eq!(rt.disk, "");
    }

    #[test]
    fn disk_on_its_own_line_does_not_squeeze_the_model() {
        // Disk has its own line now, so a present disk does NOT cut the model:
        // the model, the % and the disk all survive whole.
        let rt = pk_disk("payments", Some("sonnet"), Some(60), Some("1.2G"));
        assert_eq!(rt.disk, "1.2G");
        assert_eq!(rt.ctx_warn, "60%");
        assert_eq!(
            rt.model, "sonnet",
            "the model is unaffected by the disk token"
        );
    }

    #[test]
    fn disk_shown_even_with_no_model() {
        let rt = pk_disk("study", None, Some(60), Some("2.5G"));
        assert_eq!(rt.model, "");
        assert_eq!(rt.disk, "2.5G");
        assert_eq!(rt.ctx_warn, "60%");
    }

    #[test]
    fn thin_row_hint_is_d2() {
        // core / composite tab dropped / no pane / §6 hint "master".
        let rt = pk("core", None, None, &["master"], Some("opus"), Some(8));
        assert_eq!(rt.tab, "");
        assert_eq!(rt.pane, "");
        assert_eq!(rt.d2, "master");
        assert_eq!(rt.d3, "");
        assert_eq!(rt.model, "opus");
        assert_eq!(rt.ctx_ok, "8%");
    }

    #[test]
    fn splitter_shares_line_with_tab_when_no_pane() {
        // experiment / tab "2" / no pane / splitter "p4V".
        let rt = pk(
            "experiment",
            Some("2"),
            None,
            &["p4V"],
            Some("opus"),
            Some(5),
        );
        assert_eq!(rt.tab, "2");
        assert_eq!(rt.d2, "p4V");
        assert_eq!(rt.d3, "");
    }
}
