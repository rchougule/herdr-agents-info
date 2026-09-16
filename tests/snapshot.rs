//! Full-fleet insta snapshot of the whole owned-token map for a representative
//! fleet (see `docs/DESIGN.md`, Placement).
//!
//! Drives the real `app::gather` / `app::plans_for_sweep` pipeline through a fake
//! `HerdrClient` and captures the full 8-owned-token report for every pane in
//! a representative fleet: `study`, `Vector Search` (tab + pane), five
//! same-label `dashboard slow` Spaces (rows 3/4 share a tab but differ by
//! pane, so no splitter; the rest are distinct tabs), `lci fast` (no
//! transcript), `core` (a composite tab dropped to a thin-row hint) and five
//! `experiment` panes (E1/E2 not a collision — differ by pane; E3/E4 collide
//! on tab "2" and fall to the pane id) plus a second `core` row with both a
//! tab and a pane name.
//!
//! The `%` states and models come from synthetic transcripts staged under
//! `AGENTS_INFO_FIXTURE_DIR`. The fixed field->slot assignment (`docs/DESIGN.md`, Placement) — `$ctx_*`/`$model` on line 1, `$tab`/`$d2` on line 2, `$pane`/`$d3`
//! on line 3 — lands in one reviewable `.snap`, reconciled line-by-line against
//! the §1-A mock. Re-baseline with `cargo insta review` after an intentional
//! change.
//!
//! No cache dir is passed, so every pane is "changed" and every report is full
//! and deterministic.

use std::fs;
use std::path::{Path, PathBuf};

use agents_info::app;
use agents_info::config::Config;
use agents_info::herdr::HerdrClient;
use agents_info::model::{AgentInfo, PaneInfo, TabInfo, WorkspaceInfo};

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
        unreachable!("plans_for_sweep only builds plans; this snapshot never applies them")
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "agents-info-snap-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&d).unwrap();
    d
}

fn write_git_head(dir: &Path, branch: &str) {
    let git = dir.join(".git");
    fs::create_dir_all(&git).unwrap();
    fs::write(git.join("HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
}

/// Stage a one-line synthetic transcript for a pane: `used = pct * 2000` (the
/// 200k default window) with the given model id.
fn stage_transcript(fixture_dir: &Path, pane_id: &str, model_id: &str, pct: u64) {
    let used = pct * 2_000;
    let line = format!(
        r#"{{"type":"assistant","message":{{"model":"{model_id}","usage":{{"input_tokens":{used}}}}}}}"#
    );
    fs::write(
        fixture_dir.join(format!("{pane_id}.jsonl")),
        format!("{line}\n"),
    )
    .unwrap();
}

const OPUS: &str = "claude-opus-4-8";
const FABLE: &str = "claude-fable-1";

/// One pane of the representative fleet.
struct Spec {
    id: &'static str,
    ws_id: &'static str,
    ws: &'static str,
    tab_id: &'static str,
    tab: &'static str,
    pane_label: Option<&'static str>,
    branch: Option<&'static str>,
    /// `(model_id, pct)` or `None` for a pane with no transcript.
    usage: Option<(&'static str, u64)>,
}

#[test]
fn qa_scenario_token_map_snapshot() {
    let repos = scratch("repos");
    let fixture_dir = scratch("fx");

    // The representative fleet, in a fixed order so the
    // snapshot is stable. `dir` == workspace label for every pane (via the
    // repo-path convention below) so the cwd basename never acts as an
    // accidental distinguisher; only branch and pane presence do.
    let specs = [
        // 1: study / flow / opus / 5%.
        Spec {
            id: "ws1:p1",
            ws_id: "ws1",
            ws: "study",
            tab_id: "t1",
            tab: "flow",
            pane_label: None,
            branch: None,
            usage: Some((OPUS, 5)),
        },
        // 2: Vector Search / tab "1" + pane "vector-search-plan" / opus / 82%.
        Spec {
            id: "ws2:p1",
            ws_id: "ws2",
            ws: "Vector Search",
            tab_id: "t2",
            tab: "1",
            pane_label: Some("vector-search-plan"),
            branch: None,
            usage: Some((OPUS, 82)),
        },
        // 3/4: dashboard slow — same tab but 4 has a pane, so their identity
        // keys differ and this is NOT a collision.
        Spec {
            id: "ws3:p1",
            ws_id: "ws3",
            ws: "dashboard slow",
            tab_id: "t3",
            tab: "dashboard cache check",
            pane_label: None,
            branch: None,
            usage: Some((OPUS, 12)),
        },
        Spec {
            id: "ws4:p1",
            ws_id: "ws4",
            ws: "dashboard slow",
            tab_id: "t4",
            tab: "dashboard cache check",
            pane_label: Some("backend-caching-plan"),
            branch: None,
            usage: Some((FABLE, 18)),
        },
        // 5: dashboard slow / k8s / opus / 12%.
        Spec {
            id: "ws5:p1",
            ws_id: "ws5",
            ws: "dashboard slow",
            tab_id: "t5",
            tab: "k8s",
            pane_label: None,
            branch: None,
            usage: Some((OPUS, 12)),
        },
        // 6: dashboard slow / last enriched at fixes / opus / 34%.
        Spec {
            id: "ws6:p1",
            ws_id: "ws6",
            ws: "dashboard slow",
            tab_id: "t6",
            tab: "last enriched at fixes",
            pane_label: None,
            branch: None,
            usage: Some((OPUS, 34)),
        },
        // 7: dashboard slow / postgres analytics / fable / 29%.
        Spec {
            id: "ws7:p1",
            ws_id: "ws7",
            ws: "dashboard slow",
            tab_id: "t7",
            tab: "postgres analytics",
            pane_label: None,
            branch: None,
            usage: Some((FABLE, 29)),
        },
        // 8: lci fast / remaining lci elephants / no transcript.
        Spec {
            id: "ws8:p1",
            ws_id: "ws8",
            ws: "lci fast",
            tab_id: "t8",
            tab: "remaining lci elephants",
            pane_label: None,
            branch: None,
            usage: None,
        },
        // 9: core / composite tab (dropped) / thin -> hint "master" (default
        // branch, dir == workspace) / opus / 8%.
        Spec {
            id: "ws9:p1",
            ws_id: "ws9",
            ws: "core",
            tab_id: "t9",
            tab: "claude · 4 comments · 4 on s…",
            pane_label: None,
            branch: Some("master"),
            usage: Some((OPUS, 8)),
        },
        // 10/11: experiment, tab "1" — 11 has a pane, so this is NOT a
        // collision either.
        Spec {
            id: "wsA:p1",
            ws_id: "wsA",
            ws: "experiment",
            tab_id: "tA",
            tab: "1",
            pane_label: None,
            branch: None,
            usage: Some((FABLE, 77)),
        },
        Spec {
            id: "wsB:p1",
            ws_id: "wsB",
            ws: "experiment",
            tab_id: "tB",
            tab: "1",
            pane_label: Some("herdr-agent-info-plugin"),
            branch: None,
            usage: Some((OPUS, 42)),
        },
        // 12/13: experiment, tab "2", no pane on either — a real collision.
        // Same branch, same dir (== workspace) → both skipped → falls to the
        // pane id, "p4V" / "p4W".
        Spec {
            id: "wsC:p4V",
            ws_id: "wsC",
            ws: "experiment",
            tab_id: "tC",
            tab: "2",
            pane_label: None,
            branch: Some("master"),
            usage: Some((OPUS, 5)),
        },
        Spec {
            id: "wsD:p4W",
            ws_id: "wsD",
            ws: "experiment",
            tab_id: "tD",
            tab: "2",
            pane_label: None,
            branch: Some("master"),
            usage: Some((OPUS, 4)),
        },
        // 14: core / tab "1" + pane "worktree-fix-desktop-…" (a literal
        // 22-char label, already at the line budget — no packer truncation)
        // / opus / 26%. Different tab from row 9's (dropped) composite, so
        // no collision with it.
        Spec {
            id: "wsE:p1",
            ws_id: "wsE",
            ws: "core",
            tab_id: "tE",
            tab: "1",
            pane_label: Some("worktree-fix-desktop-…"),
            branch: None,
            usage: Some((OPUS, 26)),
        },
    ];

    let mut agents = Vec::new();
    let mut panes = Vec::new();
    let mut tabs = Vec::new();
    let mut workspaces = Vec::new();

    for s in &specs {
        // cwd basename == workspace label so the directory never acts as a
        // distinguisher (the A/B collision then correctly falls to the pane id).
        let dir = repos.join(s.id.replace(':', "_")).join(s.ws);
        fs::create_dir_all(&dir).unwrap();
        if let Some(branch) = s.branch {
            write_git_head(&dir, branch);
        }
        if let Some((model_id, pct)) = s.usage {
            stage_transcript(&fixture_dir, s.id, model_id, pct);
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
            label: s.pane_label.map(str::to_string),
            ..Default::default()
        });
        workspaces.push(WorkspaceInfo {
            workspace_id: s.ws_id.to_string(),
            label: s.ws.to_string(),
        });
        tabs.push(TabInfo {
            tab_id: s.tab_id.to_string(),
            workspace_id: s.ws_id.to_string(),
            label: s.tab.to_string(),
        });
    }

    let fake = FakeClient {
        agents,
        panes,
        tabs,
        workspaces,
    };

    // SAFETY: single test in this binary; the env var is set and removed here.
    std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &fixture_dir);
    let cfg = Config::default();
    let facts = app::gather(&fake).unwrap();
    // No cache dir → deterministic full reports for every pane. Fixed seq.
    let plans = app::plans_for_sweep(&facts, &cfg, None, None, None, 1_726_000_000_000);
    std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");

    assert_eq!(plans.len(), specs.len());
    insta::assert_json_snapshot!(plans);

    let _ = fs::remove_dir_all(&repos);
    let _ = fs::remove_dir_all(&fixture_dir);
}
