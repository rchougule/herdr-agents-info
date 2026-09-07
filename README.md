# herdr-agents-info

A [herdr](https://github.com/ogulcancelik/herdr) plugin that makes the Agents sidebar
glanceable. Today a fleet of twelve agents renders as five rows that all read
`dashboard sl…`. This plugin pushes display-only tokens per pane so each Claude Code
row shows a distinguishing name, the model, and how full the context window is.

Status: planning. See [PLAN.md](PLAN.md). No application code yet.

Scope of v1: Claude Code only. Codex and Cursor are later phases.

## How it fits together

herdr is not modified. Two layers combine:

- `~/.config/herdr/config.toml` decides where information goes (row layout, styles).
- This plugin supplies the values, via `herdr pane report-metadata --token name=...`,
  from a `[[startup]]` sweep and per-pane `[[events]]` hooks. Every run is a
  short-lived process; there is no daemon.

## Install (dev)

```bash
git clone git@github.com:rchougule/herdr-agents-info.git
cd herdr-agents-info && cargo build --release
herdr plugin link "$PWD"
```

Later, once published with the `herdr-plugin` topic:

```bash
herdr plugin install rchougule/herdr-agents-info
```

## Config recipe

Replace any existing `[ui.sidebar.agents.rows_by_agent]` block in
`~/.config/herdr/config.toml` with:

```toml
[ui.sidebar.agents.rows_by_agent]
claude = [
  # line 1: who-needs-me icon, then a bold name and a dim distinguisher
  [
    "state_icon",
    { token = "$name", bold = true, dim = false },
    { token = "$sub",  dim = true },
  ],
  # line 2: agent · model · context % (exactly one ctx_* token is ever set)
  [
    "agent",
    "$model",
    { token = "$ctx_ok",   fg = "#a6e3a1" },
    { token = "$ctx_warn", fg = "#f9e2af" },
    { token = "$ctx_hot",  fg = "#f38ba8", bold = true },
  ],
]
```

Rows that belong to other agents keep herdr's default layout. Tokens that the plugin
has not reported render as nothing, so rows collapse gracefully.

## License

TBD (private repository for now).
