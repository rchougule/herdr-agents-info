//! `agents-info` — clap CLI over the core library. Three modes:
//!   `sweep`   — full re-report of every Claude pane (startup / handoff).
//!   `enrich`  — re-report the event's target pane (+ its collision-group siblings).
//!   `doctor`  — print transcript-mapping and hook diagnostics (PLAN §9.4).

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use agents_info::app;
use agents_info::claude::{self, account::Account};
use agents_info::config::{Config, LoginMode};
use agents_info::herdr::{CliClient, HerdrClient};
use agents_info::model::EventEnvelope;
use agents_info::now_millis;

#[derive(Parser)]
#[command(
    name = "agents-info",
    version,
    about = "Glanceable Agents sidebar for herdr"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Re-report every Claude pane (startup sweep).
    Sweep,
    /// Re-report the event's target pane and its collision-group siblings.
    Enrich,
    /// Diagnose transcript mapping and hook wiring.
    Doctor,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cfg = Config::load();
    let client = CliClient::new();

    let result = match cli.command {
        Command::Sweep => run_sweep(&client, &cfg),
        Command::Enrich => run_enrich(&client, &cfg),
        Command::Doctor => return run_doctor(&client, &cfg),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // A hook failure must never affect herdr; log to stderr and exit 0
            // so the plugin runtime does not treat us as broken.
            eprintln!("agents-info: {e}");
            ExitCode::SUCCESS
        }
    }
}

/// Load the account only when the config opts in (avoids reading `~/.claude.json`
/// on every hook when the tokens are off).
fn account_for(cfg: &Config) -> Account {
    if cfg.login == LoginMode::Always {
        Account::load()
    } else {
        Account::default()
    }
}

fn run_sweep(client: &dyn HerdrClient, cfg: &Config) -> std::io::Result<()> {
    let facts = app::gather(client)?;
    let acct = account_for(cfg);
    let cache_dir = app::state_dir();

    // Phase 1: push the fast tokens (name/model/ctx + any cached disk) for every
    // pane and flush them to herdr immediately — a cold disk walk must never
    // make the sidebar feel hung.
    let phase1 = app::plans_for_sweep(
        &facts,
        cfg,
        cache_dir.as_deref(),
        acct.email.as_deref(),
        acct.org.as_deref(),
        now_millis(),
    );
    app::apply(client, &phase1)?;

    // Phase 2: measure disk off the critical path (TTL-cached, parallel,
    // timeout-bounded), then re-report only the panes whose $disk changed.
    if cfg.disk.enabled {
        app::refresh_disk(&facts, cfg, cache_dir.as_deref());
        let phase2 = app::plans_for_disk_phase2(
            &facts,
            cfg,
            cache_dir.as_deref(),
            acct.email.as_deref(),
            acct.org.as_deref(),
            now_millis(),
        );
        app::apply(client, &phase2)?;
    }
    Ok(())
}

fn run_enrich(client: &dyn HerdrClient, cfg: &Config) -> std::io::Result<()> {
    let Some(target) = resolve_target_pane() else {
        eprintln!("agents-info enrich: no target pane id in event or $HERDR_PANE_ID");
        return Ok(());
    };
    let facts = app::gather(client)?;
    let acct = account_for(cfg);
    let cache_dir = app::state_dir();
    let plans = app::plans_for_enrich(
        &facts,
        cfg,
        &target,
        cache_dir.as_deref(),
        acct.email.as_deref(),
        acct.org.as_deref(),
        now_millis(),
    );
    app::apply(client, &plans)
}

/// Resolve the enrich target: `HERDR_PLUGIN_EVENT_JSON` → `data.pane_id` /
/// `data.pane.pane_id`, falling back to `$HERDR_PANE_ID` (PLAN §4.2).
fn resolve_target_pane() -> Option<String> {
    if let Ok(json) = std::env::var("HERDR_PLUGIN_EVENT_JSON") {
        if let Ok(env) = EventEnvelope::parse(&json) {
            if let Some(id) = env.data.target_pane_id() {
                return Some(id);
            }
        }
    }
    std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|s| !s.is_empty())
}

/// `doctor`: print a human-readable diagnostic of transcript mapping and hook
/// wiring (PLAN §9.4). Read-only; never reports metadata.
fn run_doctor(client: &dyn HerdrClient, cfg: &Config) -> ExitCode {
    println!("agents-info doctor");
    println!("==================");

    // Config summary.
    println!("\n[config]");
    println!(
        "  login={} model={} width={} thresholds=({},{}) window={} auto_promote_1m={}",
        if cfg.login == LoginMode::Always {
            "always"
        } else {
            "off"
        },
        cfg.show_model,
        cfg.layout.assumed_width,
        cfg.warn,
        cfg.hot,
        cfg.ctx_default_window,
        cfg.auto_promote_1m,
    );
    println!(
        "  disk: enabled={} measure={:?} warn_mb={} refresh_secs={} timeout_ms={}",
        cfg.disk.enabled,
        cfg.disk.measure,
        cfg.disk.warn_mb,
        cfg.disk.refresh_secs,
        cfg.disk.timeout_ms,
    );

    // Environment / hook wiring.
    println!("\n[environment]");
    for key in [
        "HERDR_BIN_PATH",
        "HERDR_PLUGIN_CONFIG_DIR",
        "HERDR_PLUGIN_STATE_DIR",
        "HERDR_PLUGIN_EVENT",
        "HERDR_PANE_ID",
        "AGENTS_INFO_FIXTURE_DIR",
    ] {
        match std::env::var(key) {
            Ok(v) => println!("  {key}={v}"),
            Err(_) => println!("  {key}=(unset)"),
        }
    }

    // Account (read regardless, for diagnosis).
    let acct = Account::load();
    println!("\n[account] ~/.claude.json");
    println!("  email={}", acct.email.as_deref().unwrap_or("(none)"));
    println!("  org={}", acct.org.as_deref().unwrap_or("(none)"));

    // herdr's own Claude Code integration hook (PLAN §9.4, risk #4): without
    // it, `agent_session` is never populated and transcript mapping falls
    // back to newest-jsonl, which can pick the wrong session when two Claude
    // sessions share a cwd.
    println!("\n[claude hook] ~/.claude/settings.json");
    match claude::claude_hook_installed() {
        Some(true) => println!("  herdr-agent-state.sh: installed"),
        Some(false) => println!(
            "  herdr-agent-state.sh: NOT installed \
             (session id -> newest-jsonl fallback; may pick the wrong session \
             when two Claude sessions share a cwd — see PLAN §9 risk #4)"
        ),
        None => println!("  herdr-agent-state.sh: unknown (no $HOME, or ~/.claude/settings.json is missing/unreadable)"),
    }

    // Per-pane transcript mapping.
    println!("\n[panes] Claude transcript mapping");
    match app::gather(client) {
        Ok(facts) => {
            if facts.is_empty() {
                println!("  (no Claude panes found)");
            }
            // Run the real disk-refresh pass so the report reflects the TTL
            // cache, parallel measurement and per-tree timeout exactly as a
            // sweep would (cache-hit vs fresh, timed-out). Empty when disk is
            // off or there is no state dir.
            let cache_dir = app::state_dir();
            let probes: std::collections::HashMap<String, app::DiskProbe> =
                app::refresh_disk(&facts, cfg, cache_dir.as_deref())
                    .into_iter()
                    .map(|p| (p.pane_id.clone(), p))
                    .collect();
            for f in &facts {
                let a = &f.agent;
                let uuid = a
                    .agent_session
                    .as_ref()
                    .filter(|s| s.kind == "id")
                    .map(|s| s.value.as_str());
                let path = claude::resolve_transcript(
                    &a.pane_id,
                    a.cwd.as_deref(),
                    a.foreground_cwd.as_deref(),
                    uuid,
                );
                println!("  pane {} (ws {})", a.pane_id, a.workspace_id);
                println!(
                    "    session_id={} {}",
                    uuid.unwrap_or("(none)"),
                    if uuid.is_none() {
                        "-> newest-jsonl fallback (may pick wrong session; see PLAN risk #4)"
                    } else {
                        ""
                    }
                );
                match path {
                    Some(p) => {
                        let usage = claude::transcript::read_last_usage(&p);
                        println!("    transcript={}", p.display());
                        match usage {
                            Some(u) => {
                                let window =
                                    claude::window::resolve_window(&u.model_id, u.used, cfg);
                                let pct = claude::window::pct(u.used, window);
                                println!(
                                    "    used={} model={} ({}) window={} ({}%)",
                                    u.used,
                                    u.model_id,
                                    u.model_short(),
                                    window,
                                    pct
                                );
                            }
                            None => println!("    used=(no assistant usage yet)"),
                        }
                    }
                    None => println!("    transcript=(unresolved: no matching project dir)"),
                }

                // Disk footprint (the $disk token source), from the refresh pass
                // above so it reflects the real cache/timeout behavior.
                if !cfg.disk.enabled {
                    println!("    disk=(disabled)");
                } else if let Some(pr) = probes.get(&a.pane_id) {
                    let tgt = pr
                        .target
                        .as_deref()
                        .map(|t| t.display().to_string())
                        .unwrap_or_else(|| "(unresolved)".to_string());
                    match pr.bytes {
                        Some(bytes) => {
                            let threshold = cfg.disk.warn_mb.saturating_mul(1 << 20);
                            println!(
                                "    disk={} ({:?} {}) {}{} → token {}",
                                agents_info::disk::human_readable(bytes),
                                cfg.disk.measure,
                                tgt,
                                if pr.cache_hit { "cache-hit" } else { "fresh" },
                                if pr.timed_out { " TIMED-OUT" } else { "" },
                                if bytes >= threshold {
                                    "shown"
                                } else {
                                    "hidden"
                                },
                            );
                        }
                        None => println!(
                            "    disk=(unmeasurable{}) ({:?} {})",
                            if pr.timed_out { ", TIMED-OUT" } else { "" },
                            cfg.disk.measure,
                            tgt,
                        ),
                    }
                } else {
                    println!("    disk=(no state dir; not measured this run)");
                }
            }
        }
        Err(e) => {
            println!("  error talking to herdr: {e}");
            println!("  (is `herdr` on PATH or $HERDR_BIN_PATH set, and a server running?)");
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}
