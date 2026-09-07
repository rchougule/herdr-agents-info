//! Orchestration (§5 of `docs/naming-framing.md`): gather the full fleet
//! snapshot, compute every Claude pane's row as a pure function of it, and emit
//! full set-or-clear reports.
//!
//! `sweep` and `enrich` both fetch the whole snapshot and compute `F` for
//! **every** Claude pane. The event only decides *which* pane's transcript is
//! re-read (`ReadScope`); every other pane's `model`/`ctx` comes from the
//! per-pane cache. A report is sent only when a pane's computed `RowTokens`
//! differ from its cache (the idempotent skip, §5.2 rule 5).

use std::io;
use std::path::{Path, PathBuf};

use crate::cache::{self, PaneCache};
use crate::claude::{self, transcript::UsageEntry, window};
use crate::config::{Config, LoginMode};
use crate::git;
use crate::herdr::HerdrClient;
use crate::model::AgentInfo;
use crate::name::{self, PaneFields};
use crate::pack::{self, RowTokens};
use crate::render::{self, ReportPlan};

/// One Claude pane plus its identity-ladder inputs.
pub struct PaneFacts {
    pub agent: AgentInfo,
    pub fields: PaneFields,
}

/// Which pane(s) have their transcript re-read this invocation (§5.2 rule 1).
#[derive(Clone, Copy)]
pub enum ReadScope<'a> {
    /// `sweep`: read every pane's transcript (the startup / handoff refresh).
    All,
    /// `enrich`: read only the event's target pane; siblings use the cache.
    One(&'a str),
}

impl ReadScope<'_> {
    fn reads(&self, pane_id: &str) -> bool {
        match self {
            ReadScope::All => true,
            ReadScope::One(target) => *target == pane_id,
        }
    }
}

/// The computed outcome for one pane.
pub struct PaneOutcome {
    pub pane_id: String,
    pub tokens: RowTokens,
    /// Whether the tokens differ from the cache (i.e. a report should be sent).
    pub changed: bool,
}

fn basename(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn is_claude(a: &AgentInfo) -> bool {
    a.agent.as_deref() == Some("claude")
}

/// Read all four lists and build identity-ladder inputs for every Claude pane.
pub fn gather(client: &dyn HerdrClient) -> io::Result<Vec<PaneFacts>> {
    use std::collections::HashMap;
    let agents = client.agent_list()?;
    let workspaces = client.workspace_list()?;
    let tabs = client.tab_list()?;
    let panes = client.pane_list()?;

    let ws_label: HashMap<&str, &str> = workspaces
        .iter()
        .map(|w| (w.workspace_id.as_str(), w.label.as_str()))
        .collect();
    let tab_label: HashMap<&str, &str> = tabs
        .iter()
        .map(|t| (t.tab_id.as_str(), t.label.as_str()))
        .collect();
    let pane_label: HashMap<&str, Option<&str>> = panes
        .iter()
        .map(|p| (p.pane_id.as_str(), p.label.as_deref()))
        .collect();

    let mut out = Vec::new();
    for agent in agents.into_iter().filter(is_claude) {
        let dir = agent.foreground_cwd.as_deref().or(agent.cwd.as_deref());
        let fields = PaneFields {
            pane_id: agent.pane_id.clone(),
            workspace_label: ws_label
                .get(agent.workspace_id.as_str())
                .map(|s| s.to_string()),
            tab_label: tab_label.get(agent.tab_id.as_str()).map(|s| s.to_string()),
            pane_label: pane_label
                .get(agent.pane_id.as_str())
                .and_then(|o| o.map(str::to_string)),
            agent_name: agent.name.clone(),
            git_branch: dir.and_then(|d| git::branch_of(Path::new(d))),
            cwd_basename: dir.and_then(basename),
        };
        out.push(PaneFacts { agent, fields });
    }
    Ok(out)
}

/// The session UUID for a Claude agent (kind == "id"), if present.
fn session_uuid(a: &AgentInfo) -> Option<&str> {
    a.agent_session
        .as_ref()
        .filter(|s| s.kind == "id" && !s.value.is_empty())
        .map(|s| s.value.as_str())
}

/// Read the latest usage entry for a pane by resolving its transcript path.
fn read_usage(a: &AgentInfo) -> Option<UsageEntry> {
    let path = claude::resolve_transcript(
        &a.pane_id,
        a.cwd.as_deref(),
        a.foreground_cwd.as_deref(),
        session_uuid(a),
    )?;
    claude::transcript::read_last_usage(&path)
}

/// A pane's `(model, ctx%)`: freshly read from its transcript when in scope
/// (§5.2 rule 1), otherwise taken from the cache — never re-reading a transcript
/// for a sibling (rule 3).
fn metadata(
    agent: &AgentInfo,
    scope: ReadScope,
    cfg: &Config,
    prev: Option<&PaneCache>,
) -> (Option<String>, Option<u8>) {
    if scope.reads(&agent.pane_id) {
        match read_usage(agent) {
            Some(u) => {
                let win = window::resolve_window(&u.model_id, u.used, cfg);
                (Some(u.model_short()), Some(window::pct(u.used, win)))
            }
            None => (None, None),
        }
    } else {
        (prev.and_then(|c| c.model.clone()), prev.and_then(|c| c.pct))
    }
}

/// Compute every pane's `RowTokens` from one snapshot, reading transcripts per
/// `scope` and metadata-from-cache otherwise, and persist each pane's cache.
///
/// Identity (§3/§4/§6) is computed once over the whole fleet, so it is a pure
/// function of the snapshot — independent of which pane the event named (§5.1).
pub fn compute(
    facts: &[PaneFacts],
    cfg: &Config,
    scope: ReadScope,
    cache_dir: Option<&Path>,
) -> Vec<PaneOutcome> {
    let fields: Vec<PaneFields> = facts.iter().map(|f| f.fields.clone()).collect();
    let rows = name::compute_rows(&fields);

    facts
        .iter()
        .zip(&rows)
        .map(|(f, row)| {
            let pane_id = &f.agent.pane_id;
            let prev = cache_dir.and_then(|d| cache::load(d, pane_id));
            let (model, pct) = metadata(&f.agent, scope, cfg, prev.as_ref());

            let model_for_pack = if cfg.show_model {
                model.as_deref()
            } else {
                None
            };
            let derived: Vec<&str> = row.derived.iter().map(String::as_str).collect();
            let tokens = pack::pack(
                &row.workspace,
                row.tab.as_deref(),
                row.pane.as_deref(),
                &derived,
                model_for_pack,
                pct,
                &cfg.layout,
                cfg.warn,
                cfg.hot,
            );

            let changed = prev.as_ref().is_none_or(|p| p.tokens != tokens);
            if let Some(d) = cache_dir {
                cache::store(
                    d,
                    pane_id,
                    &PaneCache {
                        model: model.clone(),
                        pct,
                        tokens: tokens.clone(),
                    },
                );
            }

            PaneOutcome {
                pane_id: pane_id.clone(),
                tokens,
                changed,
            }
        })
        .collect()
}

/// `Some((email, org))` when the config opts in; either string may be empty.
fn login_of<'a>(
    cfg: &Config,
    email: Option<&'a str>,
    org: Option<&'a str>,
) -> Option<(&'a str, &'a str)> {
    (cfg.login == LoginMode::Always).then_some((email.unwrap_or(""), org.unwrap_or("")))
}

/// Turn changed outcomes into full reports (idempotent-skipped ones are omitted).
fn plans(
    outcomes: Vec<PaneOutcome>,
    cfg: &Config,
    email: Option<&str>,
    org: Option<&str>,
    seq: u64,
) -> Vec<ReportPlan> {
    let login = login_of(cfg, email, org);
    outcomes
        .into_iter()
        .filter(|o| o.changed)
        .map(|o| render::report_plan(&o.pane_id, &o.tokens, seq, login))
        .collect()
}

/// `sweep`: read every transcript, report every changed pane.
pub fn plans_for_sweep(
    facts: &[PaneFacts],
    cfg: &Config,
    cache_dir: Option<&Path>,
    email: Option<&str>,
    org: Option<&str>,
    seq: u64,
) -> Vec<ReportPlan> {
    let outcomes = compute(facts, cfg, ReadScope::All, cache_dir);
    plans(outcomes, cfg, email, org, seq)
}

/// `enrich`: re-read only the target pane's transcript; recompute every pane
/// from the snapshot; report every pane whose tokens changed (so a dissolving
/// collision un-splits the survivor, §5.2 rule 6).
pub fn plans_for_enrich(
    facts: &[PaneFacts],
    cfg: &Config,
    target_pane_id: &str,
    cache_dir: Option<&Path>,
    email: Option<&str>,
    org: Option<&str>,
    seq: u64,
) -> Vec<ReportPlan> {
    let outcomes = compute(facts, cfg, ReadScope::One(target_pane_id), cache_dir);
    plans(outcomes, cfg, email, org, seq)
}

/// The plugin state directory as a `PathBuf` (owned so callers can pass a ref).
pub fn state_dir() -> Option<PathBuf> {
    cache::dir()
}

/// Apply a set of plans through the client.
pub fn apply(client: &dyn HerdrClient, plans: &[ReportPlan]) -> io::Result<()> {
    for p in plans {
        client.report_metadata(&p.pane_id, &p.set, &p.clear, p.seq)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PaneInfo, TabInfo, WorkspaceInfo};
    use std::cell::RefCell;

    type RecordedReport = (String, Vec<(String, String)>, Vec<String>, u64);

    #[derive(Default)]
    struct FakeClient {
        agents: Vec<AgentInfo>,
        panes: Vec<PaneInfo>,
        tabs: Vec<TabInfo>,
        workspaces: Vec<WorkspaceInfo>,
        reports: RefCell<Vec<RecordedReport>>,
    }

    impl HerdrClient for FakeClient {
        fn agent_list(&self) -> io::Result<Vec<AgentInfo>> {
            Ok(self.agents.clone())
        }
        fn pane_list(&self) -> io::Result<Vec<PaneInfo>> {
            Ok(self.panes.clone())
        }
        fn tab_list(&self) -> io::Result<Vec<TabInfo>> {
            Ok(self.tabs.clone())
        }
        fn workspace_list(&self) -> io::Result<Vec<WorkspaceInfo>> {
            Ok(self.workspaces.clone())
        }
        fn report_metadata(
            &self,
            pane_id: &str,
            set: &[(String, String)],
            clear: &[String],
            seq: u64,
        ) -> io::Result<()> {
            self.reports.borrow_mut().push((
                pane_id.to_string(),
                set.to_vec(),
                clear.to_vec(),
                seq,
            ));
            Ok(())
        }
    }

    fn claude_agent(pane: &str, ws: &str, tab: &str) -> AgentInfo {
        AgentInfo {
            pane_id: pane.to_string(),
            workspace_id: ws.to_string(),
            tab_id: tab.to_string(),
            agent: Some("claude".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn gather_filters_to_claude_and_joins_labels() {
        let mut codex = claude_agent("w1:p2", "w1", "t1");
        codex.agent = Some("codex".to_string());
        let fake = FakeClient {
            agents: vec![claude_agent("w1:p1", "w1", "t1"), codex],
            workspaces: vec![WorkspaceInfo {
                workspace_id: "w1".into(),
                label: "dashboard".into(),
            }],
            tabs: vec![TabInfo {
                tab_id: "t1".into(),
                workspace_id: "w1".into(),
                label: "dashboard".into(),
            }],
            panes: vec![],
            ..Default::default()
        };
        let facts = gather(&fake).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(
            facts[0].fields.workspace_label.as_deref(),
            Some("dashboard")
        );
    }

    #[test]
    fn sweep_reports_one_full_plan_per_pane_without_cache() {
        let fake = FakeClient {
            agents: vec![
                claude_agent("w1:p1", "w1", "t1"),
                claude_agent("w2:p1", "w2", "t2"),
            ],
            workspaces: vec![
                WorkspaceInfo {
                    workspace_id: "w1".into(),
                    label: "dashboard".into(),
                },
                WorkspaceInfo {
                    workspace_id: "w2".into(),
                    label: "api".into(),
                },
            ],
            tabs: vec![
                TabInfo {
                    tab_id: "t1".into(),
                    workspace_id: "w1".into(),
                    label: "dashboard".into(),
                },
                TabInfo {
                    tab_id: "t2".into(),
                    workspace_id: "w2".into(),
                    label: "api".into(),
                },
            ],
            ..Default::default()
        };
        let cfg = Config::default();
        let facts = gather(&fake).unwrap();
        // No cache dir → every pane is "changed" and every report is full.
        let plans = plans_for_sweep(&facts, &cfg, None, None, None, 42);
        assert_eq!(plans.len(), 2);
        for p in &plans {
            assert_eq!(
                p.set.len() + p.clear.len(),
                15,
                "every report is full (8 owned + 7 retired)"
            );
            assert!(!p
                .set
                .iter()
                .any(|(k, _)| k == "name" || k == "sub" || k == "m1" || k == "t1"));
            assert_eq!(p.seq, 42);
        }
        apply(&fake, &plans).unwrap();
        assert_eq!(fake.reports.borrow().len(), 2);
    }
}
