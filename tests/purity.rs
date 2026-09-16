//! Purity-contract tests (`docs/DESIGN.md`, Architecture: the idempotent skip / pure-function-of-snapshot contract).
//!
//! These drive the real `app::gather` / `app::compute` / `app::plans_for_*`
//! pipeline through a fake `HerdrClient`, fixture transcripts staged under
//! `AGENTS_INFO_FIXTURE_DIR`, and a temp cache directory passed in directly (the
//! per-pane metadata cache, `$HERDR_PLUGIN_STATE_DIR` in production).
//!
//! `AGENTS_INFO_FIXTURE_DIR` is process-global, so a mutex serialises the tests
//! that set it (each uses its own unique fixture dir underneath).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agents_info::app::{self, ReadScope};
use agents_info::cache::{self, PaneCache};
use agents_info::config::Config;
use agents_info::herdr::HerdrClient;
use agents_info::model::{AgentInfo, PaneInfo, TabInfo, WorkspaceInfo};
use agents_info::pack::RowTokens;

static ENV_LOCK: Mutex<()> = Mutex::new(());

// ── fake client ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct FakeClient {
    agents: Vec<AgentInfo>,
    panes: Vec<PaneInfo>,
    tabs: Vec<TabInfo>,
    workspaces: Vec<WorkspaceInfo>,
}

impl HerdrClient for FakeClient {
    fn agent_list(&self) -> std::io::Result<Vec<AgentInfo>> {
        Ok(self.agents.clone())
    }
    fn pane_list(&self) -> std::io::Result<Vec<PaneInfo>> {
        Ok(self.panes.clone())
    }
    fn tab_list(&self) -> std::io::Result<Vec<TabInfo>> {
        Ok(self.tabs.clone())
    }
    fn workspace_list(&self) -> std::io::Result<Vec<WorkspaceInfo>> {
        Ok(self.workspaces.clone())
    }
    fn report_metadata(
        &self,
        _pane_id: &str,
        _set: &[(String, String)],
        _clear: &[String],
        _seq: u64,
    ) -> std::io::Result<()> {
        Ok(())
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

struct Spec {
    id: &'static str,
    ws_id: &'static str,
    ws: &'static str,
    tab_id: &'static str,
    tab: &'static str,
    branch: Option<&'static str>,
    fixture: Option<&'static str>,
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "agents-info-purity-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts")
}

/// Build a fake client from specs, writing `.git/HEAD` for branches and staging
/// each pane's transcript fixture. The cwd basename equals the workspace label so
/// it never accidentally acts as a distinguisher.
fn build(specs: &[Spec], repos: &Path, fixture_dir: &Path) -> FakeClient {
    let mut agents = Vec::new();
    let mut panes = Vec::new();
    let mut tabs = Vec::new();
    let mut workspaces = Vec::new();

    for s in specs {
        let dir = repos.join(s.id.replace(':', "_")).join(s.ws);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(branch) = s.branch {
            let git = dir.join(".git");
            std::fs::create_dir_all(&git).unwrap();
            std::fs::write(git.join("HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
        }
        if let Some(fixture) = s.fixture {
            std::fs::copy(
                fixtures_dir().join(fixture),
                fixture_dir.join(format!("{}.jsonl", s.id)),
            )
            .unwrap();
        }
        agents.push(AgentInfo {
            pane_id: s.id.to_string(),
            workspace_id: s.ws_id.to_string(),
            tab_id: s.tab_id.to_string(),
            agent: Some("claude".to_string()),
            cwd: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        });
        panes.push(PaneInfo {
            pane_id: s.id.to_string(),
            workspace_id: s.ws_id.to_string(),
            tab_id: s.tab_id.to_string(),
            ..Default::default()
        });
        if !workspaces
            .iter()
            .any(|w: &WorkspaceInfo| w.workspace_id == s.ws_id)
        {
            workspaces.push(WorkspaceInfo {
                workspace_id: s.ws_id.to_string(),
                label: s.ws.to_string(),
            });
        }
        if !tabs.iter().any(|t: &TabInfo| t.tab_id == s.tab_id) {
            tabs.push(TabInfo {
                tab_id: s.tab_id.to_string(),
                workspace_id: s.ws_id.to_string(),
                label: s.tab.to_string(),
            });
        }
    }
    FakeClient {
        agents,
        panes,
        tabs,
        workspaces,
    }
}

fn tokens_of<'a>(outcomes: &'a [app::PaneOutcome], id: &str) -> &'a RowTokens {
    &outcomes.iter().find(|o| o.pane_id == id).unwrap().tokens
}

// ── fixtures ──────────────────────────────────────────────────────────────────

#[test]
fn focus_event_is_not_an_input() {
    // Fixture 15: same snapshot, enrich for X then for Y → identical RowTokens
    // for every pane both times.
    let _g = ENV_LOCK.lock().unwrap();
    let repos = scratch("f15r");
    let fx = scratch("f15f");
    let cache_dir = scratch("f15c");
    let specs = [
        Spec {
            id: "w1:p1",
            ws_id: "w1",
            ws: "d",
            tab_id: "t1",
            tab: "cache",
            branch: Some("master"),
            fixture: Some("ctx_12.jsonl"),
        },
        Spec {
            id: "w2:p1",
            ws_id: "w2",
            ws: "d",
            tab_id: "t2",
            tab: "cache",
            branch: Some("dist"),
            fixture: Some("ctx_44.jsonl"),
        },
    ];
    let fake = build(&specs, &repos, &fx);
    std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &fx);
    let cfg = Config::default();
    let facts = app::gather(&fake).unwrap();

    // Seed the cache so fresh reads match cached metadata.
    app::compute(&facts, &cfg, ReadScope::All, Some(&cache_dir));

    let ax = app::compute(&facts, &cfg, ReadScope::One("w1:p1"), Some(&cache_dir));
    let ay = app::compute(&facts, &cfg, ReadScope::One("w2:p1"), Some(&cache_dir));
    std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");

    for id in ["w1:p1", "w2:p1"] {
        assert_eq!(
            tokens_of(&ax, id),
            tokens_of(&ay, id),
            "pane {id} changed with the event target"
        );
    }

    for d in [&repos, &fx, &cache_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn sibling_recompute_keeps_model() {
    // Fixture 16: a sibling Y has cached model=opus; enriching X (a collision
    // sibling, Y's transcript NOT re-read) still emits Y with its cached model
    // and exactly one ctx token. A collision appears between the seeding sweep
    // (Y unique) and the enrich (Y colliding with X), forcing Y to re-report.
    let _g = ENV_LOCK.lock().unwrap();
    let repos = scratch("f16r");
    let fx = scratch("f16f");
    let cache_dir = scratch("f16c");
    let cfg = Config::default();

    // Snapshot A: X and Y do NOT collide (different tabs). Sweep seeds the cache;
    // Y's transcript (opus, 44%) is read here and only here.
    let specs_a = [
        Spec {
            id: "w1:p1",
            ws_id: "w1",
            ws: "d",
            tab_id: "t1",
            tab: "xcache",
            branch: Some("master"),
            fixture: Some("ctx_12.jsonl"),
        },
        Spec {
            id: "w2:p1",
            ws_id: "w2",
            ws: "d",
            tab_id: "t2",
            tab: "ycache",
            branch: Some("dist"),
            fixture: Some("ctx_44.jsonl"),
        },
    ];
    let fake_a = build(&specs_a, &repos, &fx);
    std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &fx);
    let facts_a = app::gather(&fake_a).unwrap();
    app::compute(&facts_a, &cfg, ReadScope::All, Some(&cache_dir));

    // Snapshot B: Y's tab now equals X's → they collide. Enrich targets X, so Y
    // is recomputed from the cache (model=opus) without a transcript read.
    let specs_b = [
        Spec {
            id: "w1:p1",
            ws_id: "w1",
            ws: "d",
            tab_id: "t1",
            tab: "cache",
            branch: Some("master"),
            fixture: Some("ctx_12.jsonl"),
        },
        Spec {
            id: "w2:p1",
            ws_id: "w2",
            ws: "d",
            tab_id: "t2b",
            tab: "cache",
            branch: Some("dist"),
            fixture: Some("ctx_44.jsonl"),
        },
    ];
    let fake_b = build(&specs_b, &repos, &fx);
    let facts_b = app::gather(&fake_b).unwrap();
    let plans = app::plans_for_enrich(&facts_b, &cfg, "w1:p1", Some(&cache_dir), None, None, 5);
    std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");

    let y = plans
        .iter()
        .find(|p| p.pane_id == "w2:p1")
        .expect("sibling Y re-reported");
    // Y carries its cached model in $model.
    assert!(
        y.set.contains(&("model".to_string(), "opus".to_string())),
        "sibling kept its cached model: {:?}",
        y.set
    );
    // Exactly one ctx token set.
    let ctx: Vec<&str> = y
        .set
        .iter()
        .map(|(k, _)| k.as_str())
        .filter(|k| k.starts_with("ctx_"))
        .collect();
    assert_eq!(ctx.len(), 1, "exactly one ctx token: {:?}", y.set);

    for d in [&repos, &fx, &cache_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn report_is_always_full() {
    // Fixture 17 (pipeline level): every emitted report sets or clears all 12
    // owned tokens.
    let _g = ENV_LOCK.lock().unwrap();
    let repos = scratch("f17r");
    let fx = scratch("f17f");
    let cache_dir = scratch("f17c");
    let specs = [
        Spec {
            id: "w1:p1",
            ws_id: "w1",
            ws: "study",
            tab_id: "t1",
            tab: "flow",
            branch: Some("main"),
            fixture: Some("ctx_12.jsonl"),
        },
        Spec {
            id: "w2:p1",
            ws_id: "w2",
            ws: "core",
            tab_id: "t2",
            tab: "claude · 4 comments",
            branch: Some("master"),
            fixture: Some("haiku.jsonl"),
        },
    ];
    let fake = build(&specs, &repos, &fx);
    std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &fx);
    let cfg = Config::default();
    let facts = app::gather(&fake).unwrap();
    let plans = app::plans_for_sweep(&facts, &cfg, Some(&cache_dir), None, None, 9);
    std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");

    assert_eq!(plans.len(), 2);
    let owned = [
        "ctx_ok", "ctx_warn", "ctx_hot", "model", "disk", "tab", "d2", "pane", "d3", "t1", "t2",
        "t3", "d1", "mo1", "mo2", "mo3",
    ];
    for p in &plans {
        let mut keys: Vec<&str> = p
            .set
            .iter()
            .map(|(k, _)| k.as_str())
            .chain(p.clear.iter().map(String::as_str))
            .collect();
        keys.sort();
        let mut expect: Vec<&str> = owned.to_vec();
        expect.sort();
        assert_eq!(keys, expect, "pane {} is not a full report", p.pane_id);
    }

    for d in [&repos, &fx, &cache_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn identical_output_skips_report() {
    // The idempotent-skip (docs/DESIGN.md, Architecture). An `enrich` on a snapshot
    // unchanged since the last run produces zero reports — but a `sweep` always
    // re-pushes every pane, since it is the startup / restart / refresh path
    // where herdr's own display state has been reset (regression: rows stayed
    // blank on herdr restart until an event changed a pane's tokens).
    let _g = ENV_LOCK.lock().unwrap();
    let repos = scratch("f18r");
    let fx = scratch("f18f");
    let cache_dir = scratch("f18c");
    let specs = [
        Spec {
            id: "w1:p1",
            ws_id: "w1",
            ws: "study",
            tab_id: "t1",
            tab: "flow",
            branch: Some("main"),
            fixture: Some("ctx_12.jsonl"),
        },
        Spec {
            id: "w2:p1",
            ws_id: "w2",
            ws: "api",
            tab_id: "t2",
            tab: "api",
            branch: Some("main"),
            fixture: Some("ctx_44.jsonl"),
        },
    ];
    let fake = build(&specs, &repos, &fx);
    std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &fx);
    let cfg = Config::default();
    let facts = app::gather(&fake).unwrap();

    let first = app::plans_for_sweep(&facts, &cfg, Some(&cache_dir), None, None, 1);
    assert_eq!(first.len(), 2, "first sweep reports every pane");

    // An enrich over the now-warm cache honors the idempotent-skip.
    let enriched = app::plans_for_enrich(&facts, &cfg, "w1:p1", Some(&cache_dir), None, None, 2);
    assert!(
        enriched.is_empty(),
        "unchanged snapshot re-reports nothing on enrich: {enriched:?}"
    );

    // A second sweep against the same warm cache re-pushes every pane (the
    // restart / refresh contract), never skipping.
    let second = app::plans_for_sweep(&facts, &cfg, Some(&cache_dir), None, None, 3);
    std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");
    assert_eq!(
        second.len(),
        2,
        "sweep re-pushes every pane even when the cache matches: {second:?}"
    );

    for d in [&repos, &fx, &cache_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn closing_sibling_unsplits_survivor() {
    // Fixture 19: a two-row collision where one row closes → the survivor's
    // derived splitter is cleared on the next snapshot.
    let _g = ENV_LOCK.lock().unwrap();
    let repos = scratch("f19r");
    let fx = scratch("f19f");
    let cache_dir = scratch("f19c");
    let cfg = Config::default();

    // Snapshot A: X and Y collide → both get a branch splitter.
    let specs_a = [
        Spec {
            id: "w1:p1",
            ws_id: "w1",
            ws: "d",
            tab_id: "t1",
            tab: "cache",
            branch: Some("master"),
            fixture: Some("ctx_12.jsonl"),
        },
        Spec {
            id: "w2:p1",
            ws_id: "w2",
            ws: "d",
            tab_id: "t2",
            tab: "cache",
            branch: Some("dist"),
            fixture: Some("ctx_44.jsonl"),
        },
    ];
    let fake_a = build(&specs_a, &repos, &fx);
    std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &fx);
    let facts_a = app::gather(&fake_a).unwrap();
    let seeded = app::compute(&facts_a, &cfg, ReadScope::All, Some(&cache_dir));
    // Survivor X started split (no pane, so the splitter is in $d2).
    assert_eq!(tokens_of(&seeded, "w1:p1").d2, "master");

    // Snapshot B: Y has closed. Enrich fires for the closed pane (Y), X survives.
    let specs_b = [Spec {
        id: "w1:p1",
        ws_id: "w1",
        ws: "d",
        tab_id: "t1",
        tab: "cache",
        branch: Some("master"),
        fixture: Some("ctx_12.jsonl"),
    }];
    let fake_b = build(&specs_b, &repos, &fx);
    let facts_b = app::gather(&fake_b).unwrap();
    let after = app::compute(&facts_b, &cfg, ReadScope::One("w2:p1"), Some(&cache_dir));
    std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");

    let x = after.iter().find(|o| o.pane_id == "w1:p1").unwrap();
    assert!(
        x.changed,
        "survivor's row changed when the collision dissolved"
    );
    assert_eq!(x.tokens.d2, "", "survivor's splitter is cleared");
    assert_eq!(x.tokens.d3, "");
    // Metadata survives from the cache (X was not re-read).
    assert_eq!(x.tokens.model, "opus");

    for d in [&repos, &fx, &cache_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn cache_round_trip_via_state_dir() {
    // Sanity: a sweep persists a pane's metadata + tokens under the cache dir.
    let _g = ENV_LOCK.lock().unwrap();
    let repos = scratch("fcr");
    let fx = scratch("fcf");
    let cache_dir = scratch("fcc");
    let specs = [Spec {
        id: "w1:p1",
        ws_id: "w1",
        ws: "study",
        tab_id: "t1",
        tab: "flow",
        branch: Some("main"),
        fixture: Some("ctx_44.jsonl"),
    }];
    let fake = build(&specs, &repos, &fx);
    std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &fx);
    let cfg = Config::default();
    let facts = app::gather(&fake).unwrap();
    app::plans_for_sweep(&facts, &cfg, Some(&cache_dir), None, None, 1);
    std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");

    let entry: PaneCache = cache::load(&cache_dir, "w1:p1").expect("cache written");
    assert_eq!(entry.model.as_deref(), Some("opus"));
    assert_eq!(entry.pct, Some(44));

    for d in [&repos, &fx, &cache_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn custom_title_becomes_agent_name_and_overrides_pane_label() {
    // A manual `/rename` (a `custom-title` line in the transcript) is read as the
    // pane's agent name and, since an agent rename wins over a pane rename, shows
    // as `A:<title>` — never the pane label.
    let _g = ENV_LOCK.lock().unwrap();
    let fx = scratch("ctf");
    let cache_dir = scratch("ctc");
    std::fs::write(
        fx.join("w1:p1.jsonl"),
        concat!(
            r#"{"type":"custom-title","customTitle":"my-agent","sessionId":"s"}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":2,"cache_read_input_tokens":100}}}"#,
            "\n"
        ),
    )
    .unwrap();

    let fake = FakeClient {
        agents: vec![AgentInfo {
            pane_id: "w1:p1".into(),
            workspace_id: "w1".into(),
            tab_id: "t1".into(),
            agent: Some("claude".into()),
            ..Default::default()
        }],
        panes: vec![PaneInfo {
            pane_id: "w1:p1".into(),
            workspace_id: "w1".into(),
            tab_id: "t1".into(),
            label: Some("should-be-overridden".into()),
            ..Default::default()
        }],
        tabs: vec![TabInfo {
            tab_id: "t1".into(),
            workspace_id: "w1".into(),
            label: "sometab".into(),
        }],
        workspaces: vec![WorkspaceInfo {
            workspace_id: "w1".into(),
            label: "dash".into(),
        }],
    };

    std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &fx);
    let cfg = Config::default();
    let facts = app::gather(&fake).unwrap();
    let outcomes = app::compute(&facts, &cfg, ReadScope::All, Some(&cache_dir));
    std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");

    let t = tokens_of(&outcomes, "w1:p1");
    assert_eq!(t.pane, "A:my-agent");
    assert!(!t.pane.contains("should-be-overridden"));

    for d in [&fx, &cache_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
}
