# Design

How `herdr-agents-info` decides **what** to show on each Claude row and **where** it lands.
This is the contributor's map; for install and the config recipe see the
[README](../README.md), and read the cited source files for the exact detail.

The section numbers (`§2`–`§6`) are the ones the code comments cite, so they line up with
`src/name.rs` et al.

## Architecture

The plugin is a short-lived process, invoked by herdr in two modes; there is no daemon.

- **`sweep`** (startup, the "refresh all rows" action) re-reports **every** Claude pane.
- **`enrich`** (per-pane hook: focus / status / created / closed) re-reports the event's
  pane and recomputes the rest from cache.

Both fetch the **whole fleet snapshot** (`agent/workspace/tab/pane list`) and compute every
pane's row. Identity is a **pure function of the snapshot** — the event only decides whose
*transcript* is re-read, never how rows are named (`src/app.rs`). That is what makes a
collision splitter (§4) safe: a row changes only when the snapshot changes.

- **Full set-or-clear reports.** A report always carries the plugin's entire token set for
  that pane: every owned key is either set or explicitly cleared in the same call. There is
  no partial "name-only" report (`src/render.rs`).
- **Per-pane cache** (`$HERDR_PLUGIN_STATE_DIR/<pane_id>.json`, `src/cache.rs`) holds the
  last emitted tokens plus `model` / `pct` / `disk_bytes`. `enrich` reads a sibling's
  metadata from here rather than re-reading its transcript. The **idempotent skip**: if a
  pane's recomputed tokens equal the cache, `enrich` sends nothing. **`sweep` never skips**
  — it re-pushes unconditionally, because herdr drops plugin tokens on restart and the
  cache would otherwise mask a wiped display.
- **Best-effort I/O everywhere.** A hook must never take the sidebar down, so every read is
  fallible and swallowed (`src/main.rs` returns success even on error); a missing value
  clears its token rather than erroring.

## §2 Normalization

`norm` (`src/name.rs`): trim, collapse internal whitespace, strip a leading `refs/heads/`
and a trailing `.git`; empty-after-norm counts as **absent**. Comparison is `norm` then
Unicode-lowercase — no prefix / fuzzy / substring matching.

A label containing the herdr separator ` · ` is **composite** (plugin-composed status such
as `claude · 4 comments`, not a user rename) and is treated as absent.

## §3 Identity ladder

Each pane's identity is built rung by rung; a rung is dropped when it is absent or would
duplicate a rung above it.

- **Rung 0 — workspace leader** (bold, unprefixed, always present). Falls back to the cwd
  basename, then the short pane id, so a row always has a leader.
- **Rung 1 — tab.** Shown when present, not composite, and ≠ workspace. Prefixed `T:`.
- **Rung 2 — agent / pane name = `agent_name ?? pane_label`.** An **agent rename**
  (`agent start <name>` / in-session rename) **wins** over a `pane rename` label; a
  composite `pane_label` is treated as absent. One slot, one source: the chosen source does
  **not** fall through on redundancy. Prefixed `A:` when it came from the agent name, `P:`
  when from the pane label. Dropped if it equals the workspace or the tab.

**Type prefixes** (`T:`/`P:`/`A:`; workspace is unprefixed because it always leads) are
applied for *display only*, after the equality/redundancy checks run on the raw values.
Downstream key comparisons strip the prefix so it never distorts collision or dedup logic.

## §4 Collision splitter

Panes that share a displayed identity key `(workspace, tab, pane)` are a **collision** and
would render identically. For each collision group, walk the candidate ladder
**branch → cwd basename → short pane id** (`src/name.rs`): append the first candidate that
actually *splits* the group (differs across members) to every member, then recurse on any
sub-group that still collides. Branches are abbreviated (`origin/` dropped;
`feature/`→`f/`, `bugfix/`→`b/`, `release/`→`r/`). A candidate already shown on a row (by
underlying value, prefix stripped) is not appended twice. This is the only sibling-aware
step and runs over the whole fleet at once.

## §6 Thin-row hint

A **thin** row (no tab, no pane) that got no splitter gets exactly one hint — the first
available of: non-default branch ≠ workspace → cwd basename ≠ workspace → default branch
(main/master) ≠ workspace → nothing (a bare workspace beats a pane id here).

## Placement

herdr applies one static line template per Claude row and truncates on overflow; it cannot
reflow. So the **plugin** assigns every field a fixed home — only *presence* varies, never
position (`src/pack.rs`, `src/render.rs`):

```
line 1   workspace(bold)                              ← the leader has this line to itself
line 2   T:tab(normal) · d2(dim)
line 3   P:pane|A:agent(normal) · d3(dim)
line 4   [icon] disk · [icon] model(dim) · [icon] ctx_ok|ctx_warn|ctx_hot(color)
```

The workspace is herdr's own bold token on line 1, alone, so it is **never crowded or
truncated** by metadata. The metadata line (`disk · model · ctx%`) carries an optional
per-field icon (`[icons]` in the config). The **9 owned tokens** the plugin fills, always
set-or-cleared in this order: `ctx_ok`, `ctx_warn`, `ctx_hot`, `model`, `tab`, `d2`, `pane`,
`d3`, `disk`. Derived items (§4/§6) join with ` · ` and land in `d3` when a pane is present,
else `d2`. Exactly one of the three `ctx_*` is ever set, so exactly one color shows.

Widths come from an **assumed** sidebar width (herdr never reports the live width): default
26 cols → 22 usable on the identity lines (`[layout]` in the config). Only the identity
lines (tab/pane + derived) truncate — tail-cut with `…` as a last resort, a derived item
kept whole and the identity item beside it cut instead. The metadata fields are short and
sit on their own line, so they are not truncated by the plugin. Colour means urgency, weight
(bold > normal > dim) means hierarchy, so both read on light and dark themes.

## Metadata

Layered onto identity from each pane's transcript tail (`src/claude/`):

- **Model** short form (`opus` / `sonnet` / `haiku`, else the segment after `claude-`),
  from the last real assistant `usage` entry. Synthetic entries (`model: "<synthetic>"`)
  are skipped so an interrupted turn does not surface as `<synth…>` at 0%.
- **Context %** = `used / window`, rounded and clamped 0–100, colored by thresholds
  (`< warn` ok, `< hot` warn, else hot; defaults 50 / 80). The window is a
  `[context_window.by_model]` override, else the 200k default, auto-promoted to 1M once
  observed usage exceeds the default (`auto_promote_1m`) — a heuristic, since transcripts do
  not record the 1M variant.
- **Disk footprint** (`$disk`, `src/disk.rs`) — a human-readable size shown only at or above
  `warn_mb`. Measures: `cwd` (default; the worktree tree walk, where the GB live),
  `transcript` (one file), `project_dir` (a session family's `.jsonl`s). The `cwd` walk is
  expensive, so it runs only on `sweep`, off the critical path: a **two-phase sweep** reports
  the fast tokens first, then measures distinct cwds in parallel (TTL-cached via
  `refresh_secs`, per-tree `timeout_ms`) and re-reports only panes whose size changed.
  `enrich` never measures — it reads the cached size.
