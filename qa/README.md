# QA loop

This is the visual dogfood loop from `PLAN.md` §6: build the plugin, link it
into a real herdr session, recreate the collision pain with scripted fixture
data (no tokens burned, no real Claude runs), screenshot the sidebar across
the variant matrix, and judge it.

## 1. Build and link

```bash
cargo build --release
herdr plugin link "$PWD"
```

`plugin link` does not run build commands (PLAN §8.4) — always `cargo build
--release` first, and again after every code change.

## 2. Point the plugin at fixture transcripts, then (re)start herdr

`qa/scenario.sh` stages fixture transcripts under a `AGENTS_INFO_FIXTURE_DIR`
directory. That variable has to be set **in the environment herdr itself
runs in**, because the plugin's `sweep`/`enrich` hooks run as herdr
subprocesses and inherit *herdr's* environment, not the shell you happen to
run `scenario.sh` from later.

```bash
export AGENTS_INFO_FIXTURE_DIR="$PWD/qa/.state/fixture-live"
herdr   # (re)start/attach the session that will run the plugin
```

Use the same path if you export it again before running `scenario.sh` in a
different shell — it defaults to this exact path already.

## 3. Apply the sidebar recipe

Copy the whole `[ui.sidebar.agents.rows_by_agent]` block from the top-level
[`README.md`](../README.md#config-recipe) (PLAN §2.1) into
`~/.config/herdr/config.toml`, replacing the existing one. Restart herdr (or
use live handoff) to pick it up.

Confirm the startup sweep ran:

```bash
herdr plugin log list --plugin rchougule.agents-info
```

## 4. Recreate the pain

```bash
qa/scenario.sh
```

Creates 5 workspaces via the herdr CLI only (`workspace create`, `pane
report-agent`) — see the script header for the exact fleet. Every pane is
registered as a `claude` agent through `pane report-agent --agent claude`
(a custom-hook report), never a real `claude` process, so this step is free
and offline. It is idempotent in the sense that a second run without
`--teardown` refuses to duplicate the fleet.

Then trigger a re-report — either:

```bash
herdr plugin action invoke sweep --plugin rchougule.agents-info
```

or restart/hand off the herdr server (exercises the real `[[startup]]`
hook path).

Tear down when done:

```bash
qa/scenario.sh --teardown
```

## 5. Shoot

```bash
qa/shoot.sh --variant expanded-grouped-dark-w26 --app Terminal
# or, for just the sidebar region:
qa/shoot.sh --variant expanded-grouped-dark-w26 --region 0,0,260,900
```

Shots land in `qa/out/<variant>.png` (gitignored). `qa/shoot.sh --list`
prints the suggested variant names for the §6.3 matrix (expanded/collapsed,
grouped `agent_panel_sort = "spaces"` / flat sort, dark/light theme, widths
18/26/36). `shoot.sh` only captures pixels already on screen — it does not
toggle sidebar width, sort, or theme itself, because those are `config.toml`
settings with no CLI toggle and need a herdr restart/live-handoff to take
effect. Change `config.toml`, restart, then shoot once per resulting state.
See the script header for how to find your region/window coordinates
(machine- and font-size-specific) and the Screen Recording permission it
needs.

## 6. Grade and iterate

Use the PLAN §6.7 checklist against the shots. Tune caps / the abbreviation
table / thresholds / colors in `src/name.rs`, `src/claude/window.rs`, and
`config.example.toml`; re-run `cargo build --release`, re-shoot. `cargo test`
(including `tests/snapshot.rs`) should stay green through this — a change to
what gets abbreviated, capped, or colored should show as a reviewed `.snap`
diff, not a behavior change nobody looked at.

## 7. Curate

Send the before/after pair and the A/B pair (PLAN §6.5–6.6) to a human for
the layout decision. Only curated, explicitly-added shots are committed,
under `docs/qa/` at the repo root — everything under `qa/out/` is gitignored.

```bash
mkdir -p docs/qa
cp qa/out/expanded-grouped-dark-w26.png docs/qa/
git add -f docs/qa/expanded-grouped-dark-w26.png
```

## Troubleshooting

- **`scenario.sh` says herdr is not reachable** — start or attach a herdr
  session (`herdr`) before running it; it preflights with `herdr status`.
- **Sidebar rows are blank after `scenario.sh`** — confirm
  `AGENTS_INFO_FIXTURE_DIR` was exported *before* herdr started (step 2), and
  that the recipe from step 3 was applied and herdr restarted after it.
- **`shoot.sh --app` can't find a window** — the target app needs a window
  open, and the terminal/app running `shoot.sh` needs Screen Recording (and,
  the first time, Automation) permission in System Settings → Privacy &
  Security.
