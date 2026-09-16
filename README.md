# herdr-agents-info

[![CI](https://github.com/rchougule/herdr-agents-info/actions/workflows/ci.yml/badge.svg)](https://github.com/rchougule/herdr-agents-info/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![herdr ≥ 0.8.2](https://img.shields.io/badge/herdr-%E2%89%A5%200.8.2-8957e5)](https://github.com/herdrdev/herdr)

**Tell your Claude panes apart at a glance.** A [herdr](https://github.com/herdrdev/herdr)
plugin that gives every Claude Code row in the Agents sidebar a distinguishing name, its
model, and how full its context window is — so a fleet of a dozen agents stops reading as
five identical `dashboard sl…` rows.

<p align="center">
  <img src="assets/sidebar.png" width="330" alt="The Agents sidebar with a distinguishing name, model, and context % on every Claude row">
</p>

Without it, panes that share a workspace collapse into the same truncated label. This plugin
pushes display-only tokens per pane — a contextual name (branch, directory, or pane name),
the model (`opus`/`fable`/…), and a color-coded context `%` (green → amber → red as it
fills) — computed fresh from each session's transcript. herdr itself is never modified.

Scope of v1: Claude Code only. Codex and Cursor are later phases. See
[PLAN.md](PLAN.md) for the full design and [qa/README.md](qa/README.md) for the
dogfood/screenshot loop.

## How it fits together

herdr is not modified. Two layers combine:

- `~/.config/herdr/config.toml` decides where information goes (row layout, styles).
- This plugin supplies the values, via `herdr pane report-metadata --token name=...`,
  from a `[[startup]]` sweep and per-pane `[[events]]` hooks. Every run is a
  short-lived process; there is no daemon.

## Install (dev)

```bash
git clone git@github.com:rchougule/herdr-agents-info.git
cd herdr-agents-info
cargo build --release   # build first: `plugin link` does not run build commands
herdr plugin link "$PWD"
```

After any code change, `cargo build --release` again — `herdr plugin link`
only registers the manifest/binary path, it never rebuilds for you. Once
linked and enabled, apply the [config recipe](#config-recipe) below and see
[`qa/README.md`](qa/README.md) for the full dogfood loop (fixture-driven
sidebar screenshots, no real Claude sessions or tokens involved).

Later, once published with the `herdr-plugin` topic:

```bash
herdr plugin install rchougule/herdr-agents-info
```

`herdr plugin install` runs the manifest's `[[build]]` command
(`cargo build --release`) itself, so `cargo` must be on the installing
machine.

## Config recipe

Which fields are shown, when, and how they look is specified in
[`docs/naming-framing.md`](docs/naming-framing.md); *where* each field lands (the line
and slot) is specified in [`docs/layout-design.md`](docs/layout-design.md) §3, which
supersedes naming-framing §7.2/§7.4 (see that document's §4 supersedes table).
`src/name.rs`, `src/pack.rs` and `src/render.rs` implement both. Replace any existing
`[ui.sidebar.agents.rows_by_agent]` block in `~/.config/herdr/config.toml` with **the
whole block below**:

```toml
[ui.sidebar.agents.rows_by_agent]
claude = [
  ["state_icon",
    { token = "workspace",  bold = true },
    { token = "$ctx_ok",    fg = "#a6e3a1", bold = true },
    { token = "$ctx_warn",  fg = "#f9e2af", bold = true },
    { token = "$ctx_hot",   fg = "#f38ba8", bold = true },
    { token = "$model",     dim = true }],
  [{ token = "$tab",  dim = false }, { token = "$d2", dim = true }],
  [{ token = "$pane", dim = false }, { token = "$d3", dim = true }],
]
```

Every field has exactly one fixed home — line 1 is metadata, lines 2–3 are identity,
and nothing ever crosses (layout-design §1-A):

- `workspace` is herdr's built-in leader, rendered **bold**, and always leads line 1.
- `$ctx_ok`/`$ctx_warn`/`$ctx_hot` is the context `%`, the only **colored** text on a
  row — green under 50, amber 50–79, red (bold) at 80+. The plugin only ever sets one of
  the three and clears the other two, so exactly one color shows, always right after the
  workspace.
- `$model` (`opus`/`fable`/…) is always the last token of line 1, **dim**. It never
  floats to another line; if the workspace is too long for it to fit, it is tail-cut or
  cleared there — the `%` is never touched.
- `$tab` is the rung-1 tab (§3 of naming-framing.md), shown at **normal** weight
  (`dim = false`) on line 2 — its own line, always, even when it would fit on line 1.
- `$pane` is the rung-2 pane name (`pane rename` / `agent start <name>`), shown at
  **normal** weight on line 3 — its own line, always, even when it would fit on line 2.
- `$d2`/`$d3` are **derived** distinguishers — a git branch, directory, short pane id
  (§4), or the thin-row hint (§6). **Dim**. A row carries at most one of the two: `$d3`
  when the row has a pane name, else `$d2` alongside the tab (or alone, on a thin row).

Colour means urgency; weight (bold > normal > dim) means hierarchy, so both read the
same on any theme (§7.3 of naming-framing.md). Rows that belong to other agents keep
herdr's default layout, and tokens the plugin has not reported render as nothing, so
rows collapse gracefully.

**Upgrading from the old class-per-line config?** The old `$t1..3`/`$d1`/`$mo1..3`
tokens are retired; this plugin clears them on every report (belt-and-braces) so
nothing stale renders even if you haven't replaced the config block yet — but replace it
anyway, since the retired tokens are no longer set to anything.

### Row spacing

`row_gap` (blank lines between agent entries) lives under `[ui.sidebar.agents]`
(a sibling of `rows_by_agent`, not nested under it) and defaults to `0`. This plugin's
layout keeps that default: every entry's line 1 (bold, at the left margin, next to the
state icon) is already visually self-delimiting, so a gap would only spend lines
repeating information the layout already carries. On a tall sidebar with room to spare,
opt in with:

```toml
[ui.sidebar.agents]
row_gap = 1
```

### Light themes

The three hexes are the entire theme surface. The block above is catppuccin **mocha**
(the dogfood dark theme). For a light theme (catppuccin **latte**), swap the three
`$ctx_*` colours:

```toml
    { token = "$ctx_ok",   fg = "#40a02b" },
    { token = "$ctx_warn", fg = "#df8e1d" },
    { token = "$ctx_hot",  fg = "#d20f39", bold = true },
```

### Fixed field→slot layout (not class-per-line packing)

herdr applies one static line-template to every Claude row and, when a row overflows,
truncates it (tail-cut with `…`, flex tokens dropped front-first). It cannot natively
use one line when the content fits and more when it does not — but that is no longer
something this plugin asks it to do. Instead of flowing items and floating the model to
whichever line has room (the old design, and the source of the "model on line 2 on one
row, tab on line 2 on the next" inconsistency it caused — see layout-design.md's intro),
the **plugin** now assigns every field a fixed token, one home each: `$ctx_*`/`$model`
always resolve on line 1, `$tab`/`$d2` always on line 2, `$pane`/`$d3` always on line 3.
The only thing that varies is *presence*, never position. Identity is still never
truncated to make room for the model — `$model` is tail-cut or cleared on line 1 before
that would ever happen — and a field is only ever tail-truncated as a last resort, when
it cannot fit its own line whole (except a derived item, which is kept whole and the
identity item beside it is cut instead — layout-design §3.3). Empty tokens are cleared,
so herdr drops empty lines and a row that shrinks loses its stale lower lines.

**Assumed-width caveat.** herdr never tells the plugin the live sidebar width, so the
packer works off an *assumed* width (default 26 columns, `[layout] assumed_width` in the
plugin config) and derives the per-line budgets from it. On a much wider or narrower
sidebar the wrapping can be a touch conservative or a touch tight; set `assumed_width`
(and, if needed, `line1_usable` / `other_usable`) in the plugin config to retune — no
rebuild required. See `config.example.toml`.

## Known limitations (v1)

- **1M-context detection is a heuristic, not certain.** Claude Code transcripts do
  not record whether a session is running the 1M-context variant. The plugin
  auto-promotes a pane's window to 1M once observed usage exceeds the 200k default
  (`auto_promote_1m`, on by default) — correct once a session crosses 200k, but it
  shows an inflated percentage for a 1M session still under 200k. Set
  `[context_window.by_model]` in the plugin config to override per model instead of
  relying on auto-promotion. See `PLAN.md` §5.2 / §9.1.
- **Login/org tokens default off.** `$login`/`$org` are built and read from
  `~/.claude.json`, but for a single-account user the value is identical on every
  row — chart-junk under this plugin's governing rule — so `[tokens] login` defaults
  to `"off"`. Set it to `"always"` to show them; the `"auto"` mode (show only when
  values differ across rows) needs per-pane account detection and is reserved for
  P2. See `PLAN.md` §5.3.
- **Non-Claude rows are unchanged until P2.** Codex/Cursor/etc. panes keep herdr's
  default `rows` layout (`state_icon`, `workspace`, `tab`, `agent`); this plugin
  only supplies `rows_by_agent.claude`. A mixed fleet will look inconsistent across
  agent kinds until the P2 adapter trait lands. See `PLAN.md` §7 (P2), §9.9.

## License

[MIT](LICENSE).
