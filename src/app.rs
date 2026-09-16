//! Orchestration (`docs/DESIGN.md`, Architecture): gather the full fleet
//! snapshot, compute every Claude pane's row as a pure function of it, and emit
//! full set-or-clear reports.
//!
//! `sweep` and `enrich` both fetch the whole snapshot and compute `F` for
//! **every** Claude pane. The event only decides *which* pane's transcript is
//! re-read (`ReadScope`); every other pane's `model`/`ctx` comes from the
//! per-pane cache. A report is sent only when a pane's computed `RowTokens`
//! differ from its cache (the idempotent skip; see `docs/DESIGN.md`, Architecture).

use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::cache::{self, PaneCache};
use crate::claude::{self, window};
use crate::config::{Config, LoginMode};
use crate::disk::{self, Measure};
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

/// Which pane(s) have their transcript re-read this invocation (Architecture).
#[derive(Clone, Copy)]
pub enum ReadScope<'a> {
    /// `sweep`: read every pane's transcript (the startup / handoff refresh).
    All,
    /// `enrich`: read only the event's target pane; siblings use the cache.
    One(&'a str),
    /// Read no transcripts — every pane's model/ctx comes from the cache. Used
    /// by the sweep's disk phase-2 recompute, where only `disk_bytes` (freshly
    /// measured into the cache) has changed since phase 1.
    None,
}

impl ReadScope<'_> {
    fn reads(&self, pane_id: &str) -> bool {
        match self {
            ReadScope::All => true,
            ReadScope::One(target) => *target == pane_id,
            ReadScope::None => false,
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

/// Resolve a pane's transcript path (shared by the usage read and the disk
/// measure, so the path is resolved once).
fn transcript_path(a: &AgentInfo) -> Option<PathBuf> {
    claude::resolve_transcript(
        &a.pane_id,
        a.cwd.as_deref(),
        a.foreground_cwd.as_deref(),
        session_uuid(a),
    )
}

/// A pane's transcript-derived metadata for one report.
struct Meta {
    model: Option<String>,
    pct: Option<u8>,
    /// The pane's manual `/rename` title (`custom-title`), the `A:` identity
    /// source. Freshly read from the transcript when in scope, else the cache —
    /// so a sibling's identity is computed on `enrich` without re-reading it.
    custom_title: Option<String>,
    /// Raw disk footprint in bytes (the `warn_mb` gate + formatting are applied
    /// at pack time). Always read from the cache — the sweep's dedicated disk
    /// phase ([`refresh_disk`]) is the only place that measures, so no report is
    /// ever held up by a tree walk.
    disk_bytes: Option<u64>,
}

/// A pane's `(model, ctx%, custom_title)`: freshly read from its transcript when
/// in scope (Architecture) via one tail scan, otherwise from the cache.
/// `disk_bytes` always comes from the cache (never measured on a report's
/// critical path).
fn metadata(agent: &AgentInfo, scope: ReadScope, cfg: &Config, prev: Option<&PaneCache>) -> Meta {
    let (model, pct, custom_title) = if scope.reads(&agent.pane_id) {
        match transcript_path(agent)
            .as_deref()
            .map(claude::transcript::read_tail)
        {
            Some(tail) => {
                let (model, pct) = match tail.usage {
                    Some(u) => {
                        let win = window::resolve_window(&u.model_id, u.used, cfg);
                        (Some(u.model_short()), Some(window::pct(u.used, win)))
                    }
                    None => (None, None),
                };
                (model, pct, tail.custom_title)
            }
            None => (None, None, None),
        }
    } else {
        (
            prev.and_then(|c| c.model.clone()),
            prev.and_then(|c| c.pct),
            prev.and_then(|c| c.custom_title.clone()),
        )
    };
    Meta {
        model,
        pct,
        custom_title,
        disk_bytes: prev.and_then(|c| c.disk_bytes),
    }
}

/// The `$disk` token value for a pane: `Some("1.2G")` only when disk reporting
/// is on and the footprint is at or above the `warn_mb` threshold; else `None`
/// (cleared). Applied at pack time so a `warn_mb` change takes effect on the
/// next compute for every pane, without a re-measure.
fn disk_token(bytes: Option<u64>, cfg: &Config) -> Option<String> {
    if !cfg.disk.enabled {
        return None;
    }
    let threshold = cfg.disk.warn_mb.saturating_mul(1 << 20);
    bytes.filter(|&b| b >= threshold).map(disk::human_readable)
}

/// Compute every pane's `RowTokens` from one snapshot, reading transcripts per
/// `scope` and metadata-from-cache otherwise, and persist each pane's cache.
///
/// Identity (§3/§4/§6) is computed once over the whole fleet, so it is a pure
/// function of the snapshot — independent of which pane the event named (Architecture).
pub fn compute(
    facts: &[PaneFacts],
    cfg: &Config,
    scope: ReadScope,
    cache_dir: Option<&Path>,
) -> Vec<PaneOutcome> {
    // Resolve each pane's metadata first (one transcript read per in-scope pane;
    // cache otherwise). This must precede identity, because the manual `/rename`
    // title lives in the transcript and feeds the rung-2 `A:` name.
    let resolved: Vec<(Option<PaneCache>, Meta)> = facts
        .iter()
        .map(|f| {
            let prev = cache_dir.and_then(|d| cache::load(d, &f.agent.pane_id));
            let meta = metadata(&f.agent, scope, cfg, prev.as_ref());
            (prev, meta)
        })
        .collect();

    // Build the effective identity inputs: the agent name is the manual title
    // when present, else herdr's `agent start`/rename name (agent rename wins →
    // `A:`). Identity is then a pure function of this assembled snapshot.
    let fields: Vec<PaneFields> = facts
        .iter()
        .zip(&resolved)
        .map(|(f, (_prev, meta))| {
            let mut fld = f.fields.clone();
            fld.agent_name = meta.custom_title.clone().or(fld.agent_name);
            fld
        })
        .collect();
    let rows = name::compute_rows(&fields);

    facts
        .iter()
        .zip(&rows)
        .zip(&resolved)
        .map(|((f, row), (prev, meta))| {
            let pane_id = &f.agent.pane_id;

            let model_for_pack = if cfg.show_model {
                meta.model.as_deref()
            } else {
                None
            };
            let disk_str = disk_token(meta.disk_bytes, cfg);
            let derived: Vec<&str> = row.derived.iter().map(String::as_str).collect();
            let tokens = pack::pack(
                row.tab.as_deref(),
                row.pane.as_deref(),
                &derived,
                model_for_pack,
                meta.pct,
                disk_str.as_deref(),
                &cfg.layout,
                &cfg.icons,
                cfg.warn,
                cfg.hot,
            );

            let changed = prev.as_ref().is_none_or(|p| p.tokens != tokens);
            if let Some(d) = cache_dir {
                cache::store(
                    d,
                    pane_id,
                    &PaneCache {
                        model: meta.model.clone(),
                        pct: meta.pct,
                        custom_title: meta.custom_title.clone(),
                        disk_bytes: meta.disk_bytes,
                        // Preserve the measurement timestamp — compute never
                        // measures; only refresh_disk sets it.
                        disk_measured_at: prev.as_ref().and_then(|p| p.disk_measured_at),
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

/// Turn outcomes into full reports. When `force` is false the idempotent-skip
/// (the idempotent skip) omits panes whose tokens match the cache; when `force` is true
/// every pane is reported regardless — used by `sweep`, whose whole job is to
/// re-push the fleet after herdr's own display state has been reset (a restart
/// or the manual "refresh all rows" action), where the persisted cache no longer
/// reflects what herdr is showing.
fn plans(
    outcomes: Vec<PaneOutcome>,
    cfg: &Config,
    email: Option<&str>,
    org: Option<&str>,
    seq: u64,
    force: bool,
) -> Vec<ReportPlan> {
    let login = login_of(cfg, email, org);
    outcomes
        .into_iter()
        .filter(|o| force || o.changed)
        .map(|o| render::report_plan(&o.pane_id, &o.tokens, seq, login))
        .collect()
}

/// `sweep`: read every transcript and unconditionally re-report **every** pane
/// (the idempotent-skip is bypassed). Sweep is the startup / handoff / "refresh
/// all rows" path, where herdr's display has been reset but the persisted cache
/// still holds the last tokens — skipping there would leave every row blank
/// until an event happened to change its tokens.
pub fn plans_for_sweep(
    facts: &[PaneFacts],
    cfg: &Config,
    cache_dir: Option<&Path>,
    email: Option<&str>,
    org: Option<&str>,
    seq: u64,
) -> Vec<ReportPlan> {
    let outcomes = compute(facts, cfg, ReadScope::All, cache_dir);
    plans(outcomes, cfg, email, org, seq, true)
}

/// One pane's disk-measurement outcome, for diagnostics (`doctor`).
pub struct DiskProbe {
    pub pane_id: String,
    /// The path measured (cwd dir, or transcript file), when resolvable.
    pub target: Option<PathBuf>,
    pub bytes: Option<u64>,
    /// Served from the TTL cache without measuring this sweep.
    pub cache_hit: bool,
    /// The (cwd) walk hit its `timeout_ms` budget; the last-known size is kept.
    pub timed_out: bool,
}

/// Resolve the path a pane's disk footprint is measured over, per the mode: the
/// working directory (worktree) for `cwd`, else the resolved transcript file.
fn measure_target(agent: &AgentInfo, cfg: &Config) -> Option<PathBuf> {
    match cfg.disk.measure {
        Measure::Cwd => agent
            .foreground_cwd
            .as_deref()
            .or(agent.cwd.as_deref())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from),
        Measure::Transcript | Measure::ProjectDir => transcript_path(agent),
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The sweep's disk-measurement pass — run *between* the two report phases so no
/// report ever waits on a tree walk. For every pane whose cached footprint is
/// stale, it measures the target once (distinct targets in parallel via
/// `thread::scope`, each bounded by `timeout_ms`) and writes `disk_bytes` +
/// `disk_measured_at` back into the cache, preserving the rest of the entry.
///
/// Only the expensive `cwd` walk is TTL-gated (`refresh_secs`); the cheap
/// `transcript` / `project_dir` measures refresh every sweep. A timed-out walk
/// keeps the last-known size and stamps `now`, so it is retried only after the
/// TTL rather than every sweep. No-op when disk is disabled or there is no cache
/// dir. Returns a probe per pane for `doctor`.
pub fn refresh_disk(facts: &[PaneFacts], cfg: &Config, cache_dir: Option<&Path>) -> Vec<DiskProbe> {
    let Some(dir) = cache_dir else {
        return Vec::new();
    };
    if !cfg.disk.enabled {
        return Vec::new();
    }
    let now = unix_secs();
    let ttl = cfg.disk.refresh_secs;
    let ttl_gated = cfg.disk.measure.is_tree_walk();

    struct Pending {
        pane_id: String,
        target: Option<PathBuf>,
        prev: Option<PaneCache>,
        stale: bool,
    }
    let pend: Vec<Pending> = facts
        .iter()
        .map(|f| {
            let pane_id = f.agent.pane_id.clone();
            let prev = cache::load(dir, &pane_id);
            let target = measure_target(&f.agent, cfg);
            let fresh = ttl_gated
                && prev
                    .as_ref()
                    .and_then(|c| c.disk_measured_at)
                    .is_some_and(|t| now.saturating_sub(t) < ttl);
            let stale = target.is_some() && !fresh;
            Pending {
                pane_id,
                target,
                prev,
                stale,
            }
        })
        .collect();

    // Distinct stale targets, each measured exactly once.
    let unique: Vec<PathBuf> = {
        let mut seen = std::collections::HashSet::new();
        pend.iter()
            .filter(|p| p.stale)
            .filter_map(|p| p.target.clone())
            .filter(|t| seen.insert(t.clone()))
            .collect()
    };
    let measure = cfg.disk.measure;
    let timeout = Duration::from_millis(cfg.disk.timeout_ms);
    let mut sized: std::collections::HashMap<PathBuf, disk::Sized> =
        std::collections::HashMap::new();
    thread::scope(|scope| {
        let handles: Vec<_> = unique
            .iter()
            .map(|t| {
                let t = t.clone();
                scope.spawn(move || {
                    let deadline = Instant::now() + timeout;
                    (t.clone(), disk::measure(&t, measure, deadline))
                })
            })
            .collect();
        for h in handles {
            if let Ok((t, s)) = h.join() {
                sized.insert(t, s);
            }
        }
    });

    pend.into_iter()
        .map(|p| {
            let prev_bytes = p.prev.as_ref().and_then(|c| c.disk_bytes);
            let prev_at = p.prev.as_ref().and_then(|c| c.disk_measured_at);
            let (bytes, cache_hit, timed_out, measured_at) = if !p.stale {
                (prev_bytes, true, false, prev_at)
            } else {
                match p.target.as_ref().and_then(|t| sized.get(t)) {
                    Some(s) if s.timed_out => (prev_bytes, false, true, Some(now)),
                    Some(s) => (s.bytes, false, false, Some(now)),
                    None => (prev_bytes, true, false, prev_at),
                }
            };
            if p.stale {
                let mut c = p.prev.clone().unwrap_or_default();
                c.disk_bytes = bytes;
                c.disk_measured_at = measured_at;
                cache::store(dir, &p.pane_id, &c);
            }
            DiskProbe {
                pane_id: p.pane_id,
                target: p.target,
                bytes,
                cache_hit,
                timed_out,
            }
        })
        .collect()
}

/// Sweep phase 2: after [`refresh_disk`] has updated the cache, recompute every
/// pane from the cache (no transcript re-read — `ReadScope::None`) and report
/// only those whose tokens changed, i.e. panes whose freshly-measured `$disk`
/// differs from what phase 1 reported. Idempotent, unlike phase 1's forced push.
pub fn plans_for_disk_phase2(
    facts: &[PaneFacts],
    cfg: &Config,
    cache_dir: Option<&Path>,
    email: Option<&str>,
    org: Option<&str>,
    seq: u64,
) -> Vec<ReportPlan> {
    let outcomes = compute(facts, cfg, ReadScope::None, cache_dir);
    plans(outcomes, cfg, email, org, seq, false)
}

/// `enrich`: re-read only the target pane's transcript; recompute every pane
/// from the snapshot; report every pane whose tokens changed (so a dissolving
/// collision un-splits the survivor).
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
    plans(outcomes, cfg, email, org, seq, false)
}

/// The plugin state directory as a `PathBuf` (owned so callers can pass a ref).
pub fn state_dir() -> Option<PathBuf> {
    cache::dir()
}

/// Apply a set of plans through the client. Resilient per pane: a single pane's
/// failing report is logged and skipped, never aborting the rest — one bad
/// report must not blank every pane after it in the sweep.
pub fn apply(client: &dyn HerdrClient, plans: &[ReportPlan]) -> io::Result<()> {
    for p in plans {
        if let Err(e) = client.report_metadata(&p.pane_id, &p.set, &p.clear, p.seq) {
            eprintln!("agents-info: report for pane {} failed: {e}", p.pane_id);
        }
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
                10,
                "every report is full (10 owned tokens, within herdr's 16-token cap)"
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

    /// Regression: after a herdr restart, herdr's own pane display is wiped but
    /// the plugin's on-disk cache persists with the last-emitted tokens. Sweep
    /// must re-push every pane regardless — the idempotent-skip would otherwise
    /// leave every row blank until an event changed its tokens (the reported bug:
    /// "info doesn't populate until I do something in the session").
    #[test]
    fn sweep_reports_every_pane_even_when_cache_matches() {
        use std::path::PathBuf;

        let cache_dir: PathBuf = std::env::temp_dir().join(format!(
            "agents-info-sweep-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&cache_dir).unwrap();

        let fake = FakeClient {
            agents: vec![claude_agent("w1:p1", "w1", "t1")],
            workspaces: vec![WorkspaceInfo {
                workspace_id: "w1".into(),
                label: "dashboard".into(),
            }],
            tabs: vec![TabInfo {
                tab_id: "t1".into(),
                workspace_id: "w1".into(),
                label: "dashboard".into(),
            }],
            ..Default::default()
        };
        let cfg = Config::default();
        let facts = gather(&fake).unwrap();

        // Prime the cache exactly as a prior run would have (simulating the
        // state that survives a restart), then confirm the token set is stable.
        let first = plans_for_sweep(&facts, &cfg, Some(&cache_dir), None, None, 1);
        assert_eq!(first.len(), 1);

        // Steady-state enrich now skips (tokens match the warm cache) …
        let enriched = plans_for_enrich(&facts, &cfg, "w1:p1", Some(&cache_dir), None, None, 2);
        assert!(
            enriched.is_empty(),
            "enrich must honor the idempotent-skip on an unchanged fleet"
        );

        // … but a restart sweep against that same warm cache must still report.
        let after_restart = plans_for_sweep(&facts, &cfg, Some(&cache_dir), None, None, 3);
        assert_eq!(
            after_restart.len(),
            1,
            "sweep must re-push every pane even when the cache matches"
        );

        let _ = std::fs::remove_dir_all(&cache_dir);
    }
}
