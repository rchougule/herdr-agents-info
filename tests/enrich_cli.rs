//! Fake-herdr integration test (PLAN §8.2).
//!
//! A shell shim stands in for the `herdr` binary via `$HERDR_BIN_PATH`: it
//! replays canned `agent/workspace/tab/pane list` JSON and records every
//! `pane report-metadata` argv to a file. The test drives the real `agents-info`
//! binary in `enrich` mode and asserts the exact reported tokens, including the
//! two cleared `ctx_*` keys and a monotonic `--seq`.
//!
//! This is a first, focused end-to-end assertion against the public `render`/CLI
//! contract. TODO(next subagent, PLAN §8.2): expand into the full fleet matrix
//! (5×dashboard collision, haiku, no-usage, 1M) and pair with the insta snapshot
//! of the whole token map for the QA scenario.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

const SHIM: &str = r#"#!/bin/sh
case "$1 $2" in
"agent list")
  echo '{"id":"x","result":{"type":"agent_list","agents":[{"pane_id":"w1:p1","terminal_id":"t","workspace_id":"w1","tab_id":"t1","agent":"claude","agent_status":"idle","focused":false,"revision":1}]}}'
  ;;
"workspace list")
  echo '{"id":"x","result":{"type":"workspace_list","workspaces":[{"workspace_id":"w1","number":1,"label":"dashboard","focused":false,"pane_count":1,"tab_count":1,"active_tab_id":"t1","agent_status":"idle"}]}}'
  ;;
"tab list")
  echo '{"id":"x","result":{"type":"tab_list","tabs":[{"tab_id":"t1","workspace_id":"w1","number":1,"label":"dashboard","focused":false,"pane_count":1,"agent_status":"idle"}]}}'
  ;;
"pane list")
  echo '{"id":"x","result":{"type":"pane_list","panes":[{"pane_id":"w1:p1","terminal_id":"t","workspace_id":"w1","tab_id":"t1","focused":false,"revision":1}]}}'
  ;;
"pane report-metadata")
  shift 2
  printf '%s\n' "$@" >> "$SHIM_RECORD"
  ;;
esac
"#;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "agents-info-it-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn enrich_reports_exact_argv_for_target_pane() {
    let dir = scratch();

    // 1. Write the shim and mark it executable.
    let shim = dir.join("fake-herdr");
    fs::write(&shim, SHIM).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();

    // 2. Fixture transcript for pane w1:p1: reuse the 44% fixture under the
    //    pane-id name the fixture-dir override expects.
    let fx = dir.join("fixtures");
    fs::create_dir_all(&fx).unwrap();
    let src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts/ctx_44.jsonl");
    fs::copy(&src, fx.join("w1:p1.jsonl")).unwrap();

    let record = dir.join("record.txt");

    // 3. Drive the real binary in enrich mode.
    let bin = env!("CARGO_BIN_EXE_agents-info");
    let status = Command::new(bin)
        .arg("enrich")
        .env("HERDR_BIN_PATH", &shim)
        .env("SHIM_RECORD", &record)
        .env("AGENTS_INFO_FIXTURE_DIR", &fx)
        .env("HERDR_PANE_ID", "w1:p1")
        // Ensure no stray plugin config affects defaults.
        .env_remove("HERDR_PLUGIN_CONFIG_DIR")
        .env_remove("HERDR_PLUGIN_EVENT_JSON")
        .status()
        .unwrap();
    assert!(status.success());

    // 4. Assert the recorded report-metadata argv.
    let recorded = fs::read_to_string(&record).unwrap();
    let args: Vec<&str> = recorded.lines().collect();

    // Everything up to (and excluding) --seq is deterministic.
    let seq_pos = args
        .iter()
        .position(|a| *a == "--seq")
        .expect("argv must contain --seq");
    let head = &args[..seq_pos];
    // Fixed field->slot token contract (layout-design §3.1): every report sets
    // or clears all 8 owned tokens plus the 7 retired class-per-line keys
    // (belt-and-braces, §3.4). Here workspace=dashboard, the tab echoes it
    // (dropped), there is no cwd/branch/pane name so the row is thin with no
    // splitter and no hint (tab/pane/d2/d3 all empty). The transcript is the
    // 44% opus fixture, so `model` is anchored on line 1 beside the colored
    // `ctx_ok`, and every other owned + retired token is explicitly cleared.
    assert_eq!(
        head,
        &[
            "w1:p1",
            "--source",
            "plugin:rchougule.agents-info",
            "--token",
            "ctx_ok=44%",
            "--token",
            "model=opus",
            "--clear-token",
            "ctx_warn",
            "--clear-token",
            "ctx_hot",
            "--clear-token",
            "tab",
            "--clear-token",
            "d2",
            "--clear-token",
            "pane",
            "--clear-token",
            "d3",
            "--clear-token",
            "t1",
            "--clear-token",
            "t2",
            "--clear-token",
            "t3",
            "--clear-token",
            "d1",
            "--clear-token",
            "mo1",
            "--clear-token",
            "mo2",
            "--clear-token",
            "mo3",
        ]
    );

    // --seq is present and is unix millis (a large positive integer).
    let seq: u64 = args[seq_pos + 1].parse().expect("seq must be numeric");
    assert!(seq > 1_700_000_000_000, "seq should be unix millis: {seq}");

    let _ = fs::remove_dir_all(&dir);
}
