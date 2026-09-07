//! Identity ladder (§3), fleet collision splitter (§4), thin-row hint (§6) and
//! the `norm` normalisation (§2) from `docs/naming-framing.md` — the canonical
//! spec this module implements. When code and doc disagree, the doc wins.
//!
//! A Claude row's *displayed* content is a pure function of the whole fleet
//! snapshot: identity is computed per pane (§3), then the collision splitter and
//! the thin-row hint (the only sibling-dependent step) run over every pane
//! together. Metadata (model / ctx%) is layered on later by the packer.

use std::collections::HashSet;

/// The herdr token separator (§2): space, middle dot, space. herdr and its
/// plugins compose tab / pane titles with it; users never type it. A label
/// containing it is plugin-composed status, not identity, and is treated as
/// **absent** (§2, §8e).
pub const SEP: &str = " · ";

/// Per-pane raw inputs. All strings are raw; `norm` (§2) is applied internally
/// before every comparison and before display.
#[derive(Debug, Clone, Default)]
pub struct PaneFields {
    /// Full pane id, e.g. `w1:p4T`.
    pub pane_id: String,
    /// Workspace label (`workspace list`) — rung 0, the bold leader.
    pub workspace_label: Option<String>,
    /// Tab label (`tab list`) — rung 1.
    pub tab_label: Option<String>,
    /// Explicit `pane rename` label (`pane list` `.label`) — rung 2 (highest intent).
    pub pane_label: Option<String>,
    /// herdr agent name (`agent start <name>`) — rung 2 fallback.
    pub agent_name: Option<String>,
    /// Git branch of `foreground_cwd ?? cwd` — derived splitter / hint only.
    pub git_branch: Option<String>,
    /// Basename of `foreground_cwd ?? cwd` — derived splitter / hint only.
    pub cwd_basename: Option<String>,
}

/// A pane's computed display row: the bold leader plus the ordered typed
/// (rungs 1–2) and derived (splitter §4 / hint §6) items the packer receives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayRow {
    pub pane_id: String,
    /// Rung 0, always present (with a fallback so a row always has a leader).
    pub workspace: String,
    /// Rung 1: the tab, when shown.
    pub tab: Option<String>,
    /// Rung 2: pane name (`pane_label ?? agent_name`), when shown.
    pub pane: Option<String>,
    /// Derived items appended by §4 (splitters) or §6 (thin-row hint).
    pub derived: Vec<String>,
}

impl DisplayRow {
    /// The typed identity items in ladder order (`[tab?, pane?]`), for the packer.
    pub fn typed(&self) -> Vec<&str> {
        let mut v = Vec::new();
        if let Some(t) = &self.tab {
            v.push(t.as_str());
        }
        if let Some(p) = &self.pane {
            v.push(p.as_str());
        }
        v
    }
}

// ── §2 normalisation ────────────────────────────────────────────────────────

/// A label is *composite* when it contains the herdr separator ` · ` (§2).
pub fn is_composite(s: &str) -> bool {
    s.contains(SEP)
}

/// `norm` for display (§2): trim; collapse internal whitespace runs to a single
/// space; strip a leading `refs/heads/`; strip a trailing `.git`. Case is kept.
/// `None` when empty after normalisation (counts as absent).
pub fn norm_display(s: &str) -> Option<String> {
    let mut v: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(rest) = v.strip_prefix("refs/heads/") {
        v = rest.to_string();
    }
    if let Some(rest) = v.strip_suffix(".git") {
        v = rest.to_string();
    }
    (!v.is_empty()).then_some(v)
}

/// Comparison key (§2): `norm_display` then Unicode-lowercase. `None` when absent.
pub fn norm_key(s: &str) -> Option<String> {
    norm_display(s).map(|v| v.to_lowercase())
}

/// Equality (§2) between two already-present display strings: `norm(a) == norm(b)`,
/// case-insensitive. No prefix / fuzzy / substring matching.
fn same(a: &str, b: &str) -> bool {
    matches!((norm_key(a), norm_key(b)), (Some(x), Some(y)) if x == y)
}

fn field_display(o: &Option<String>) -> Option<String> {
    o.as_deref().and_then(norm_display)
}

/// `p4T` from `w1M:p4T` (the segment after the last `:`).
pub fn short_pane_id(pane_id: &str) -> String {
    pane_id
        .trim()
        .rsplit(':')
        .next()
        .unwrap_or(pane_id)
        .to_string()
}

/// Branch abbreviations (§4): drop `origin/`, then `feature/`→`f/`,
/// `bugfix/`→`b/`, `release/`→`r/`.
pub fn abbreviate(s: &str) -> String {
    let mut s = s.to_string();
    if let Some(rest) = s.strip_prefix("origin/") {
        s = rest.to_string();
    }
    const TABLE: &[(&str, &str)] = &[("feature/", "f/"), ("bugfix/", "b/"), ("release/", "r/")];
    for (from, to) in TABLE {
        if let Some(rest) = s.strip_prefix(from) {
            s = format!("{to}{rest}");
            break;
        }
    }
    s
}

fn is_default_branch(branch: &str) -> bool {
    matches!(norm_key(branch).as_deref(), Some("main") | Some("master"))
}

// ── §3 identity ladder ────────────────────────────────────────────────────────

/// Rung 0: the bold leader. Always present; falls back to the cwd basename then
/// the short pane id so a row always has a leader (§3).
fn leader(p: &PaneFields) -> String {
    field_display(&p.workspace_label)
        .or_else(|| field_display(&p.cwd_basename))
        .unwrap_or_else(|| short_pane_id(&p.pane_id))
}

/// Rung 1: the tab, shown when present, **not composite**, and ≠ workspace (§3).
fn tab_item(p: &PaneFields, workspace: &str) -> Option<String> {
    let t = field_display(&p.tab_label)?;
    if is_composite(&t) || same(&t, workspace) {
        return None;
    }
    Some(t)
}

/// Rung 2: `pane_label ?? agent_name` (§3). A **composite** `pane_label` is
/// treated as absent (it is plugin-composed status, not a user rename), so it
/// falls through to `agent_name`; `agent_name` is never composite. Once a source
/// is taken it does **not** fall through on redundancy — one slot, one source.
fn pane_item(p: &PaneFields, workspace: &str, tab: Option<&str>) -> Option<String> {
    let source = match field_display(&p.pane_label) {
        Some(pl) if !is_composite(&pl) => Some(pl),
        // composite pane_label → absent → fall through to the agent name
        _ => field_display(&p.agent_name),
    };
    let c = source?;
    if same(&c, workspace) {
        return None;
    }
    if let Some(t) = tab {
        if same(&c, t) {
            return None;
        }
    }
    Some(c)
}

// ── the fleet computation (§3 → §4 → §6) ──────────────────────────────────────

/// Compute every Claude pane's display row over the whole snapshot. This is the
/// only sibling-aware step; it is a pure function of `panes` (§5).
pub fn compute_rows(panes: &[PaneFields]) -> Vec<DisplayRow> {
    let mut rows: Vec<DisplayRow> = panes
        .iter()
        .map(|p| {
            let workspace = leader(p);
            let tab = tab_item(p, &workspace);
            let pane = pane_item(p, &workspace, tab.as_deref());
            DisplayRow {
                pane_id: p.pane_id.clone(),
                workspace,
                tab,
                pane,
                derived: Vec::new(),
            }
        })
        .collect();

    resolve_collisions(&mut rows, panes);
    apply_thin_hints(&mut rows, panes);
    rows
}

/// A row's displayed identity key (§4): `(norm(ws), norm(tab), norm(pane))`.
type IdentityKey = (Option<String>, Option<String>, Option<String>);

/// A partition key while resolving a collision: identity key + normalised derived.
type PartitionKey = (Vec<Option<String>>, Vec<String>);

/// The displayed identity key of a row (§4): `(norm(ws), norm(tab), norm(pane))`.
fn identity_key(row: &DisplayRow) -> IdentityKey {
    (
        norm_key(&row.workspace),
        row.tab.as_deref().and_then(norm_key),
        row.pane.as_deref().and_then(norm_key),
    )
}

/// The partition key while resolving a collision: identity key + the normalised
/// derived items appended so far.
fn partition_key(row: &DisplayRow) -> PartitionKey {
    let (w, t, p) = identity_key(row);
    (
        vec![w, t, p],
        row.derived.iter().filter_map(|d| norm_key(d)).collect(),
    )
}

/// The already-shown items on a row (typed + derived), as comparison keys.
fn shown_keys(row: &DisplayRow) -> HashSet<String> {
    row.tab
        .iter()
        .chain(row.pane.iter())
        .chain(row.derived.iter())
        .filter_map(|s| norm_key(s))
        .collect()
}

#[derive(Clone, Copy)]
enum Candidate {
    Branch,
    Dir,
    PaneId,
}

const CANDIDATES: [Candidate; 3] = [Candidate::Branch, Candidate::Dir, Candidate::PaneId];

impl Candidate {
    /// The candidate's display value for a pane (branch abbreviated), or `None`
    /// when the pane has no such value. The pane id is always present.
    fn value(self, p: &PaneFields) -> Option<String> {
        match self {
            Candidate::Branch => field_display(&p.git_branch).map(|b| abbreviate(&b)),
            Candidate::Dir => field_display(&p.cwd_basename),
            Candidate::PaneId => Some(short_pane_id(&p.pane_id)),
        }
    }
}

/// §4: group every pane by displayed identity key; resolve each collision group.
fn resolve_collisions(rows: &mut [DisplayRow], panes: &[PaneFields]) {
    // First-seen order preserved so output is deterministic.
    let mut groups: Vec<(IdentityKey, Vec<usize>)> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let key = identity_key(row);
        if let Some(slot) = groups.iter_mut().find(|(k, _)| *k == key) {
            slot.1.push(i);
        } else {
            groups.push((key, vec![i]));
        }
    }
    for (_, members) in &groups {
        if members.len() >= 2 {
            resolve(rows, panes, members, 0);
        }
    }
}

/// Walk the candidate ladder (§4): the first candidate that splits the group is
/// appended to every eligible member; recurse on still-colliding subgroups.
fn resolve(rows: &mut [DisplayRow], panes: &[PaneFields], members: &[usize], start: usize) {
    for (ci, &c) in CANDIDATES.iter().enumerate().skip(start) {
        // A candidate identical across the whole group (or absent everywhere)
        // does not split it — appending it would be noise. Skip it.
        let distinct: HashSet<Option<String>> = members
            .iter()
            .map(|&i| c.value(&panes[i]).and_then(|v| norm_key(&v)))
            .collect();
        if distinct.len() <= 1 {
            continue;
        }

        // Append the candidate to every member that has it and is not already
        // showing it.
        for &i in members {
            if let Some(v) = c.value(&panes[i]) {
                let Some(vk) = norm_key(&v) else { continue };
                if !shown_keys(&rows[i]).contains(&vk) {
                    rows[i].derived.push(v);
                }
            }
        }

        // Partition by identity key + appended items; recurse on sub-collisions.
        let mut subs: Vec<(PartitionKey, Vec<usize>)> = Vec::new();
        for &i in members {
            let key = partition_key(&rows[i]);
            if let Some(slot) = subs.iter_mut().find(|(k, _)| *k == key) {
                slot.1.push(i);
            } else {
                subs.push((key, vec![i]));
            }
        }
        for (_, sg) in &subs {
            if sg.len() >= 2 {
                resolve(rows, panes, sg, ci + 1);
            }
        }
        return;
    }
}

/// §6: a thin row (no tab, no pane) that received no splitter gets exactly one
/// hint — the first of: non-default branch → dir ≠ ws → default branch → nothing.
fn apply_thin_hints(rows: &mut [DisplayRow], panes: &[PaneFields]) {
    for (i, p) in panes.iter().enumerate() {
        let row = &mut rows[i];
        let thin = row.tab.is_none() && row.pane.is_none();
        if !thin || !row.derived.is_empty() {
            continue;
        }
        if let Some(hint) = thin_hint(p, &row.workspace) {
            row.derived.push(hint);
        }
    }
}

fn thin_hint(p: &PaneFields, ws: &str) -> Option<String> {
    let branch = field_display(&p.git_branch);
    let branch_disp = branch.as_deref().map(abbreviate);
    let default = branch.as_deref().map(is_default_branch).unwrap_or(false);
    let dir = field_display(&p.cwd_basename);

    // 1. non-default branch ≠ workspace
    if let Some(b) = &branch_disp {
        if !default && !same(b, ws) {
            return Some(b.clone());
        }
    }
    // 2. directory ≠ workspace
    if let Some(d) = &dir {
        if !same(d, ws) {
            return Some(d.clone());
        }
    }
    // 3. default branch ≠ workspace
    if let Some(b) = &branch_disp {
        if default && !same(b, ws) {
            return Some(b.clone());
        }
    }
    // 4. nothing (never a pane id — §8g)
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builder for a pane with sensible empties.
    #[derive(Default)]
    struct P {
        id: &'static str,
        ws: &'static str,
        tab: Option<&'static str>,
        pane_label: Option<&'static str>,
        agent_name: Option<&'static str>,
        branch: Option<&'static str>,
        dir: Option<&'static str>,
    }
    impl P {
        fn build(self) -> PaneFields {
            PaneFields {
                pane_id: self.id.to_string(),
                workspace_label: Some(self.ws.to_string()),
                tab_label: self.tab.map(str::to_string),
                pane_label: self.pane_label.map(str::to_string),
                agent_name: self.agent_name.map(str::to_string),
                git_branch: self.branch.map(str::to_string),
                cwd_basename: self.dir.map(str::to_string),
            }
        }
    }

    fn rows(panes: Vec<PaneFields>) -> Vec<DisplayRow> {
        compute_rows(&panes)
    }

    // ── §2 normalisation ──────────────────────────────────────────────────────

    #[test]
    fn norm_collapses_and_strips() {
        assert_eq!(norm_display("  a   b ").as_deref(), Some("a b"));
        assert_eq!(
            norm_display("refs/heads/feature/x").as_deref(),
            Some("feature/x")
        );
        assert_eq!(norm_display("proj.git").as_deref(), Some("proj"));
        assert_eq!(norm_display("   ").as_deref(), None);
    }

    #[test]
    fn equality_is_case_and_whitespace_insensitive() {
        assert!(same("Dashboard", " dashboard "));
        assert!(!same("Vector Search", "vector-search-plan"));
    }

    #[test]
    fn composite_detection() {
        assert!(is_composite("claude · 4 comments · 4 on screen"));
        assert!(!is_composite("dashboard cache check"));
    }

    #[test]
    fn abbreviation_table_and_origin_drop() {
        assert_eq!(abbreviate("feature/auth"), "f/auth");
        assert_eq!(abbreviate("bugfix/crash"), "b/crash");
        assert_eq!(abbreviate("release/1.2"), "r/1.2");
        assert_eq!(abbreviate("origin/feature/auth"), "f/auth");
        assert_eq!(abbreviate("origin/main"), "main");
    }

    // ── Ladder and redundancy (fixtures 1–4 + composite pane) ──────────────────

    #[test]
    fn pane_label_survives_branch() {
        // Fixture 1: pane backend-cache, branch master, tab present → typed
        // contains backend-cache; derived empty.
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "dash",
            tab: Some("sometab"),
            pane_label: Some("backend-cache"),
            branch: Some("master"),
            dir: Some("dash"),
            ..P::default()
        }
        .build()]);
        assert_eq!(r[0].pane.as_deref(), Some("backend-cache"));
        assert!(r[0].typed().contains(&"backend-cache"));
        assert!(r[0].derived.is_empty());
    }

    #[test]
    fn tab_equal_workspace_dropped() {
        // Fixture 2: tab == workspace (case/whitespace differing) → no tab.
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "Dashboard",
            tab: Some("  dashboard "),
            ..P::default()
        }
        .build()]);
        assert_eq!(r[0].tab, None);
        assert!(r[0].typed().is_empty());
    }

    #[test]
    fn composite_tab_dropped_then_hint() {
        // Fixture 3: tab `claude · 4 comments`, no pane → thin → hint (branch).
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "core",
            tab: Some("claude · 4 comments"),
            branch: Some("auth-mw"),
            dir: Some("core"),
            ..P::default()
        }
        .build()]);
        assert_eq!(r[0].tab, None);
        assert!(r[0].typed().is_empty());
        assert_eq!(r[0].derived, vec!["auth-mw".to_string()]);
    }

    #[test]
    fn pane_slot_does_not_fall_through() {
        // Fixture 4: pane_label == tab, agent_name present → neither shown.
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "w",
            tab: Some("auth"),
            pane_label: Some("auth"),
            agent_name: Some("other"),
            ..P::default()
        }
        .build()]);
        assert_eq!(r[0].tab.as_deref(), Some("auth"));
        assert_eq!(r[0].pane, None);
        assert_eq!(r[0].typed(), vec!["auth"]);
    }

    #[test]
    fn composite_pane_label_dropped() {
        // Coordinator addendum: a composite pane_label is treated as absent and
        // falls through to agent_name (which is never composite).
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "w",
            tab: Some("t"),
            pane_label: Some("claude · 4 comments · 4 on screen"),
            agent_name: Some("myagent"),
            ..P::default()
        }
        .build()]);
        assert_eq!(r[0].pane.as_deref(), Some("myagent"));
        assert!(!r[0].typed().iter().any(|s| s.contains(SEP)));

        // With no agent_name, a composite pane_label leaves the slot empty.
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "w",
            tab: Some("t"),
            pane_label: Some("claude · 4 comments"),
            ..P::default()
        }
        .build()]);
        assert_eq!(r[0].pane, None);
        assert_eq!(r[0].typed(), vec!["t"]);
    }

    // ── Collision (fixtures 5–10) ──────────────────────────────────────────────

    #[test]
    fn collision_branch_splits_all_members() {
        // Fixture 5: two rows same key, branches master/dist → both get branch.
        let r = rows(vec![
            P {
                id: "w1:p1",
                ws: "d",
                tab: Some("cache"),
                branch: Some("master"),
                ..P::default()
            }
            .build(),
            P {
                id: "w2:p1",
                ws: "d",
                tab: Some("cache"),
                branch: Some("dist"),
                ..P::default()
            }
            .build(),
        ]);
        assert_eq!(r[0].derived, vec!["master".to_string()]);
        assert_eq!(r[1].derived, vec!["dist".to_string()]);
    }

    #[test]
    fn collision_same_branch_skipped() {
        // Fixture 6: both master, dirs differ → no branch; both get dir basename.
        let r = rows(vec![
            P {
                id: "w1:p1",
                ws: "d",
                tab: Some("cache"),
                branch: Some("master"),
                dir: Some("dir-a"),
                ..P::default()
            }
            .build(),
            P {
                id: "w2:p1",
                ws: "d",
                tab: Some("cache"),
                branch: Some("master"),
                dir: Some("dir-b"),
                ..P::default()
            }
            .build(),
        ]);
        assert_eq!(r[0].derived, vec!["dir-a".to_string()]);
        assert_eq!(r[1].derived, vec!["dir-b".to_string()]);
        assert!(!r.iter().any(|x| x.derived.contains(&"master".to_string())));
    }

    #[test]
    fn collision_falls_to_pane_id() {
        // Fixture 7: same key, same branch, same dir → both get pane id.
        let r = rows(vec![
            P {
                id: "w1:p1",
                ws: "d",
                tab: Some("cache"),
                branch: Some("master"),
                dir: Some("d"),
                ..P::default()
            }
            .build(),
            P {
                id: "w1:p2",
                ws: "d",
                tab: Some("cache"),
                branch: Some("master"),
                dir: Some("d"),
                ..P::default()
            }
            .build(),
        ]);
        assert_eq!(r[0].derived, vec!["p1".to_string()]);
        assert_eq!(r[1].derived, vec!["p2".to_string()]);
    }

    #[test]
    fn collision_recurses_on_subgroup() {
        // Fixture 8: branches master/master/dist → dist gets only dist; the two
        // master rows get master AND the next splitter (their differing dirs).
        let r = rows(vec![
            P {
                id: "w1:p1",
                ws: "d",
                tab: Some("cache"),
                branch: Some("master"),
                dir: Some("a"),
                ..P::default()
            }
            .build(),
            P {
                id: "w2:p1",
                ws: "d",
                tab: Some("cache"),
                branch: Some("master"),
                dir: Some("b"),
                ..P::default()
            }
            .build(),
            P {
                id: "w3:p1",
                ws: "d",
                tab: Some("cache"),
                branch: Some("dist"),
                dir: Some("c"),
                ..P::default()
            }
            .build(),
        ]);
        assert_eq!(r[0].derived, vec!["master".to_string(), "a".to_string()]);
        assert_eq!(r[1].derived, vec!["master".to_string(), "b".to_string()]);
        assert_eq!(r[2].derived, vec!["dist".to_string()]);
    }

    #[test]
    fn no_collision_no_splitter() {
        // Fixture 9: unique key, branch present → derived empty.
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "d",
            tab: Some("cache"),
            branch: Some("master"),
            ..P::default()
        }
        .build()]);
        assert!(r[0].derived.is_empty());
    }

    #[test]
    fn splitter_not_duplicated() {
        // Fixture 10: tab `auth-mw`, branch `auth-mw`, collision → branch skipped
        // on that row (already shown as the tab).
        let r = rows(vec![
            P {
                id: "w1:p1",
                ws: "w",
                tab: Some("auth-mw"),
                branch: Some("auth-mw"),
                ..P::default()
            }
            .build(),
            P {
                id: "w2:p1",
                ws: "w",
                tab: Some("auth-mw"),
                branch: Some("other"),
                ..P::default()
            }
            .build(),
        ]);
        assert!(r[0].derived.is_empty());
        assert_eq!(r[1].derived, vec!["other".to_string()]);
    }

    // ── Thin hint (fixtures 11–14) ─────────────────────────────────────────────

    #[test]
    fn thin_hint_prefers_feature_branch() {
        // Fixture 11: no tab/pane, branch auth-mw → auth-mw.
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "ws",
            branch: Some("auth-mw"),
            ..P::default()
        }
        .build()]);
        assert_eq!(r[0].derived, vec!["auth-mw".to_string()]);
    }

    #[test]
    fn thin_hint_default_branch_yields_to_dir() {
        // Fixture 12: branch main, dir dash-v2 ≠ ws → dash-v2.
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "dashboard",
            branch: Some("main"),
            dir: Some("dash-v2"),
            ..P::default()
        }
        .build()]);
        assert_eq!(r[0].derived, vec!["dash-v2".to_string()]);
    }

    #[test]
    fn thin_hint_default_branch_last() {
        // Fixture 13: branch main, dir == ws → main.
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "core",
            branch: Some("main"),
            dir: Some("core"),
            ..P::default()
        }
        .build()]);
        assert_eq!(r[0].derived, vec!["main".to_string()]);
    }

    #[test]
    fn thin_hint_never_pane_id() {
        // Fixture 14: no branch, dir == ws → derived empty (never a pane id).
        let r = rows(vec![P {
            id: "w1:p1",
            ws: "core",
            dir: Some("core"),
            ..P::default()
        }
        .build()]);
        assert!(r[0].derived.is_empty());
    }
}
