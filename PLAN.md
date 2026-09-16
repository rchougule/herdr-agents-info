# herdr-agents-info — Plan

> **Historical.** This is the original design plan. Parts of the layout design
> (notably the `name`/`sub` role-swap in §2 and Appendix B config) were later
> superseded — the shipped behavior is specified in
> [`docs/naming-framing.md`](docs/naming-framing.md) and
> [`docs/layout-design.md`](docs/layout-design.md). Kept for design history.

Plugin id: `rchougule.agents-info`. Language: Rust (single binary, two modes). Target
host: herdr 0.8.2 (`Cargo.toml:3` in the herdr source tree).

Every API claim below was checked against the herdr 0.8.2 source tree on 2026-09-07.
References are `path:line`. herdr itself is not modified by this project.

---

## 1. Product thesis and jobs to be done

### The pain

herdr's Agents sidebar shows the workspace label and tab label per row. With ~12
agents spread over workspaces that share a label, five rows read `dashboard sl…` and
the user cannot tell which one is which without clicking through. The sidebar is
26 columns wide by default (min 18, max 36; `src/config/model.rs:851-855,1104`), so
there is very little room and names already truncate.

### Jobs to be done, ranked

| # | Job | Today | This plugin |
|---|-----|-------|-------------|
| 1 | **Disambiguation** — which agent is this row? | fails when labels collide | `name` + `sub` tokens with a collision-aware distinguisher. THE win. |
| 2 | Who needs me? | solved by `state_icon` | keep `state_icon` first on line 1, untouched |
| 3 | Is the context window nearly full? | invisible | `ctx_ok` / `ctx_warn` / `ctx_hot` token, colored |
| 4 | Which model is it running? | invisible | `model` token (`opus` / `sonnet` / `haiku`) |
| 5 | Is it stale? | partially (state) | `ago` token, phase 1.5 |

### The governing rule

> Add only what disambiguates or is actionable; kill anything that reads the same
> on every row.

Consequences baked into the design:

- Login/org (`user@example.com` / `Example Org`) is identical on every row for a
  single-account user. It is chart-junk there. Built, but **default off** (section 5.3).
- When workspace labels collide, the **distinguisher must lead** the name; otherwise
  every row still reads identically after truncation (section 5.1).
- `agent` on line 2 is redundant when the layout is already Claude-only. It stays in
  the default because mixed fleets (Codex + Claude) are the norm, but Candidate B in
  the A/B (section 6) drops it.

### The single biggest product bet

That a two-line row of `[icon] name · sub` / `agent · model · ctx%` is enough for the
user to pick the right agent out of twelve in under a second, without clicking. If the
screenshot QA in section 6 shows rows still reading alike, the plan's fallback is to
make the distinguisher *always* lead (not only on collision), not to add more tokens.

---

## 2. Proposed sidebar layout

Default Claude row, two lines, at the default 26-column sidebar (24 usable columns on
line 1, 22 on line 2 after herdr's indent; see section 3.4):

```
 ● auth-mw · dashboard
   claude · opus · 44%
 ◌ billing · dashboard
   claude · sonnet · 91%
 ● main · api
   claude · haiku · 12%
```

- Line 1: `state_icon`, then `$name` (bold, un-dimmed), then `$sub` (dim).
- Line 2: `agent`, `$model`, and exactly one of `$ctx_ok` / `$ctx_warn` / `$ctx_hot`.

### 2.1 The `config.toml` recipe

Drop-in replacement for the existing `[ui.sidebar.agents.rows_by_agent]` block in
`~/.config/herdr/config.toml` (the current line there is
`claude = [["state_icon", "workspace", "tab"], ["terminal_title_stripped"]]`):

```toml
[ui.sidebar.agents.rows_by_agent]
claude = [
  [
    "state_icon",
    { token = "$name", bold = true, dim = false },
    { token = "$sub",  dim = true },
  ],
  [
    "agent",
    "$model",
    { token = "$ctx_ok",   fg = "#a6e3a1" },
    { token = "$ctx_warn", fg = "#f9e2af" },
    { token = "$ctx_hot",  fg = "#f38ba8", bold = true },
  ],
]
```

Why each detail is the way it is (all verified):

- `rows_by_agent` keys must be canonical agent ids; `claude` is one
  (`src/config/sidebar.rs:354-370`, `src/detect/mod.rs:118-121`). Non-Claude rows keep
  `rows` (default `[["state_icon","workspace","tab"],["agent"]]`,
  `src/config/sidebar.rs:390-405`).
- Custom tokens are `$name`; names are 1–32 ASCII alnum/`_`/`-`
  (`src/config/sidebar.rs:187-208`). Max 16 rows and 16 tokens per row
  (`src/config/sidebar.rs:7-8`).
- Per-token style is `{ token, fg = "#rgb|#rrggbb", bold, dim }`
  (`src/config/sidebar.rs:96-101,152-162`). Custom tokens render with herdr's
  *dim* secondary style by default (`src/ui/sidebar.rs:1519,1524-1530`), and
  `dim = false` explicitly strips the DIM modifier (`src/ui/sidebar.rs:1199-1204`), so
  `$name` needs both `bold = true` and `dim = false`.
- A token the plugin has not reported is simply omitted, and a row with no visible
  tokens is dropped (`src/ui/sidebar/tokens.rs:76-86`). That is what makes the
  three-way `ctx_*` trick work: the plugin sets exactly one and clears the other two,
  and the ` · ` separator is only drawn between *visible* tokens
  (`src/ui/sidebar.rs:1120-1127`, `src/ui/sidebar/tokens.rs:144-152`).
- Token styles are static per config entry; herdr has no value-conditional color.
  Three tokens with three colors is the only way to color by threshold.
- Colors are catppuccin green/yellow/red (the default theme is `catppuccin`). Theme
  contrast in light mode is a QA item; fixed hex colors are the one theme-dependent
  choice in the recipe, which is why `$name` uses `bold`/`dim` and no `fg`.

---

## 3. How it works

### 3.1 Two layers, no herdr changes

1. **Config layer** — `[ui.sidebar.agents]` in `config.toml` declares *where* things
   go: rows of tokens, per-agent via `rows_by_agent`, per-token style. Built-in agent
   tokens: `state_icon`, `state_text`, `workspace`, `tab`, `pane`, `agent`,
   `terminal_title`, `terminal_title_stripped` (`src/config/sidebar.rs:288-297`).
2. **Plugin layer** — supplies the *values*. Pane tokens reported through
   `herdr pane report-metadata` are exposed as `$name` in Agent sidebar rows
   (`docs/next/website/src/content/docs/cli-reference.mdx:283`,
   `socket-api.mdx:773`).

### 3.2 The report call (verified)

```
$HERDR_BIN_PATH pane report-metadata <pane_id> \
  --source plugin:rchougule.agents-info \
  --token name=auth-mw --token sub=dashboard \
  --token model=opus --token ctx_warn=44% \
  --clear-token ctx_ok --clear-token ctx_hot \
  --seq <unix_millis>
```

Contract (`cli-reference.mdx:265-285`, `socket-api.mdx:745-789`):

- `--token NAME=VALUE` patches one key; `--clear-token NAME` removes one; unmentioned
  keys are untouched. Values are trimmed, control chars removed, capped at 80 chars.
  Empty value clears the key.
- Max 16 token keys per report, 32 retained per pane. We use 6 in v1.
- `--seq N`: reports with seq ≤ last accepted from the same `--source` are ignored.
  We pass unix milliseconds so concurrent hook invocations cannot regress a newer
  value. A pane accepts sequenced reports from at most 32 distinct sources for its
  lifetime — we always use the single fixed source string, so one slot.
- `--ttl-ms` optional (1..86400000). v1 omits it; tokens live until replaced or the
  pane closes. Token metadata is *not* restored after a server restart
  (`socket-api.mdx:787`) — that is exactly what the startup sweep is for.
- `--display-agent`, `--title`, `--state-label` exist and are guarded by `--agent`;
  v1 does not use them (the `agent` built-in already prints `claude`).
- `--source` must be ≤ 80 chars of ASCII letters/digits/`:`/`.`/`_`/`-`;
  `plugin:rchougule.agents-info` is valid.

### 3.3 Reading pane facts (verified)

`herdr agent list` / `herdr agent get <pane_id>` print JSON (`cli-reference.mdx:8,
291-293`). `AgentInfo` (`src/api/schema/agents.rs:184-223`) gives us:
`pane_id`, `workspace_id`, `tab_id`, `agent` (`"claude"`), `name` (herdr agent name,
if started with `agent start`), `agent_status`, `cwd`, `foreground_cwd`,
`terminal_title_stripped`, `tokens`, `focused`, and `agent_session`.

- `agent_session` is `{ source, agent, kind: "id"|"path", value }`
  (`src/api/schema/agents.rs:226-231`, `src/agent_resume.rs:16-19`). For Claude the
  kind is always `id` and the value is the Claude session id — herdr's own Claude hook
  reports `agent_session_id = hook_input.session_id`
  (`src/integration/assets/claude/herdr-agent-state.sh:62-85`) and
  `session_ref_from_report` maps every agent except `pi`/`omp` to `Id`
  (`src/agent_resume.rs:53-70`). The transcript path the hook also sends is dropped
  by herdr, so we derive the path ourselves (section 5.2).
- Labels for the name composition: pane label from `pane get`
  (`PaneInfo.label`, `src/api/schema/panes.rs:458`), tab label from `tab get`
  (`src/api/schema/tabs.rs:44`), workspace label from `workspace list`
  (`src/api/schema/workspaces.rs:55`). `AgentInfo` does not carry labels, so a sweep
  makes 1 + 1 + 1 list calls (`agent list`, `workspace list`, `tab list`) and zero
  per-pane get calls; `pane list` supplies pane labels in one call too.

### 3.4 How herdr lays out a row (verified; this drives the name design)

`src/ui/sidebar.rs:1012-1160` and `:1522-1534`:

- Line 1 is indented 1 column, later lines 3 columns. Body width is sidebar width
  minus the divider column, minus 1 more when a scrollbar shows. At the default 26,
  line 1 has ~24 usable columns and line 2 ~22.
- Separator is `" "` after `state_icon` and `" · "` (3 cols) elsewhere.
- If the row overflows, herdr first tries to fit whole tokens, dropping flexible
  (text) tokens from the **front** and re-enabling from the **end**; then remaining
  width is handed out **round-robin one column at a time** to visible text tokens,
  and each token is cut at its **tail** with `…` (`src/ui/text.rs:11-23`).

Implications the plugin must honour:

1. The **prefix** of each token is what survives. Distinguishing characters must
   come first inside `name`.
2. Two long tokens on one line get roughly equal budgets. Keep `sub` short
   (≤ 10 chars) so `name` keeps most of the line.
3. Budget on line 1: 24 − 1 (icon) − 1 (space) − 3 (` · `) = 19 columns for
   `name` + `sub`. Plugin caps: `name` ≤ 14, `sub` ≤ 10, both trimmed with our own
   `…` so truncation is deterministic and testable rather than left to the renderer.
4. Line 2 budget: 22 − `claude`(6) − 3 − `sonnet`(6) − 3 = 4 for `ctx`. `100%` is
   4 chars. Fits at the default width; at 18 columns `model` is what gets squeezed,
   which is acceptable (QA item).

---

## 4. Trigger design (locked: startup sweep + event hooks, no daemon)

### 4.1 Evidence

`PLUGIN_HOOK_EVENT_KINDS` (`src/api/schema/events.rs:286-309`) lists the events that
may run a plugin `[[events]]` hook. It includes `pane.created`, `pane.closed`,
`pane.focused`, `pane.moved`, `pane.exited`, `pane.agent_detected`,
`pane.agent_status_changed` and the workspace/tab/worktree lifecycle events. A unit
test (`events.rs:351-358`) asserts the high-volume `pane.output_changed`,
`pane.updated`, `layout.updated`, and `workspace.metadata_updated` are excluded, so
hook fan-out is bounded by user-visible state transitions, not terminal output.

Startup hooks run "once for each enabled plugin after Herdr restores the session and
its API socket is ready" and "again when a new server takes over during live
handoff", asynchronously, non-supervised, one-shot (`plugins.mdx:233-249`). Event
hooks receive `HERDR_PLUGIN_EVENT` and `HERDR_PLUGIN_EVENT_JSON`
(`plugins.mdx:257-260`; env built in `src/app/api/plugins/runtime.rs:58-62`).

### 4.2 Design

One binary, invoked as:

| Manifest entry | Command | Work |
|---|---|---|
| `[[startup]]` | `agents-info sweep` | `agent list` + label lists → enrich every Claude pane → one report per pane. Populates the sidebar right after restart / handoff, when token metadata has been wiped. |
| `[[events]] on = "pane.agent_status_changed"` | `agents-info enrich` | Re-read that pane's transcript (ctx %, model) and re-report. This is the "agent came to rest" moment. |
| `[[events]] on = "pane.agent_detected"` | `agents-info enrich` | Claude just appeared in a pane: compose name, first ctx read. |
| `[[events]] on = "pane.created"` | `agents-info enrich` | New pane may collide with existing labels: recompute names for the collision group. |
| `[[events]] on = "pane.focused"` | `agents-info enrich` | Cheap refresh of the row the user is looking at. |
| `[[events]] on = "pane.closed"` | `agents-info enrich` | Target's tokens auto-clear; siblings may no longer collide, so recompute names for the group. |

Mode `enrich` resolves the target pane from `HERDR_PLUGIN_EVENT_JSON` and falls back
to `HERDR_PANE_ID`. The env JSON is a serialized `EventEnvelope { event, data }`
(`events.rs:362-365`, `runtime.rs:241`); `data` is tagged with `type` and carries
`pane_id` for focused/closed/agent_detected/agent_status_changed and a full
`pane: PaneInfo` for created (`events.rs:493-547`). The parser reads
`data.pane_id ?? data.pane.pane_id ?? $HERDR_PANE_ID`. The exact serialized string
form of `event` (dotted vs snake_case) is a dev-phase check via a logging hook and
`herdr plugin log list`; the parser must not depend on it.

`enrich` always fetches `agent list` (one call) rather than `agent get`, because the
name composer needs the sibling set to detect collisions. It reads a transcript only
for the target pane; for colliding siblings it re-reports `name`/`sub` only (no I/O).

Every invocation is a short-lived process that exits in tens of milliseconds: two or
three `herdr` CLI spawns plus one bounded file read. No pidfile, no orphan risk, no
supervision, and a hook failure never affects herdr (`plugins.mdx:239`).

### 4.3 The honest trade-off

Context % updates when the agent's status changes (idle ↔ working ↔ blocked), i.e.
when it comes to rest — which is when the user glances at the sidebar. It does **not**
tick frame-by-frame during a long turn. A long-lived daemon subscribing to
`events.subscribe` or polling transcripts could add live mid-turn %; it would reuse
the identical enrichment code and only swap the trigger. Listed as an optional
phase-2 add-on, not v1. Reason: the marginal value (watching a number climb) is low
against the cost (a supervised process, orphan handling on handoff, and herdr's own
documented preference for one-shot hooks).

### 4.4 Ordering and races

Focus and status-change hooks can fire together. Both would report the same values;
`--seq <unix_millis>` guarantees the later one wins and a stale one is ignored
(`cli-reference.mdx:285`). No locking is needed because reports are idempotent.

---

## 5. Data dimensions (v1, Claude Code)

All three come from local files the user already has; no network, no credentials.

### 5.1 Contextual name → tokens `name` (bold) + `sub` (dim)

Inputs per pane: pane label (explicit `pane rename`), tab label, workspace label,
`cwd` / `foreground_cwd`, git branch, herdr agent `name`, `agent_status`, `pane_id`.

Candidates, in order, each cleaned (trim, collapse whitespace, strip `.git`):

```
C1 pane label            (user typed it: highest intent)
C2 herdr agent name      (agent start <name>)
C3 tab label             only if it differs from the workspace label
C4 workspace label
D1 git branch of foreground_cwd ?? cwd   (read .git/HEAD directly; worktree-aware
                                          via the `.git` file → gitdir; fallback:
                                          `gitBranch` field in the transcript)
D2 basename(foreground_cwd ?? cwd)       if it differs from C4
D3 short pane id         (`p3` from `w1:p3`)
D4 "<agent> · <status>"  last resort
```

Algorithm (`name.rs`), deterministic and unit-tested with fixtures:

1. `primary = first non-empty of C1..C4`; `distinguisher = first of D1..D3 that is
   non-empty and ≠ primary`.
2. Group Claude panes by `primary`. If a group has ≥ 2 members, **for every member of
   the group** swap roles: `name = distinguisher`, `sub = primary`. This is the
   product-critical rule: the differing text must lead so it survives the
   tail-truncation described in 3.4.
3. Within a group, if `distinguisher` still collides (five `dashboard` worktrees all
   on `main`), walk D2 → D3 until each member is unique; D3 always is.
4. If there is no collision: `name = primary`, `sub = distinguisher` (may be empty;
   an empty `sub` is cleared, and herdr omits it and its separator).
5. Cap `name` to 14 and `sub` to 10 display columns with a trailing `…`; abbreviate
   common prefixes first (`feature/` → `f/`, `bugfix/` → `b/`, `release/` → `r/`,
   `origin/` dropped). Abbreviations are a table in `name.rs`, tuned by the QA loop.

Worked example, the real pain (5 × `dashboard`, worktrees on different branches):

| pane | workspace | branch | → name | → sub |
|---|---|---|---|---|
| w1:p1 | dashboard | auth-mw | `auth-mw` | `dashboard` |
| w2:p1 | dashboard | billing | `billing` | `dashboard` |
| w3:p1 | dashboard | main | `main` | `dashboard` |
| w4:p1 | dashboard | main (2nd) | `dash-v2` (cwd basename) | `dashboard` |
| w5:p1 | api | main | `api` | `main` |

### 5.2 Context window % → exactly one of `ctx_ok` / `ctx_warn` / `ctx_hot`; plus `model`

Transcript location: `~/.claude/projects/<cwd-slug>/<session-id>.jsonl`, where
`cwd-slug` is the pane cwd with every `/` replaced by `-` (e.g.
`/home/user/proj/herdr-agents-info` → `-home-user-proj-herdr-agents-info`).

Pane → transcript mapping, in order:

1. `agent_session.value` (Claude session UUID; section 3.3) →
   `<slug(cwd)>/<uuid>.jsonl`. Also try `slug(foreground_cwd)`.
2. If the file is missing (session started outside herdr's hook, or the hook is not
   installed), fall back to the newest `*.jsonl` (by mtime) under `slug(cwd)`, then
   `slug(foreground_cwd)`.
3. If no directory exists: report nothing for `ctx_*`/`model` (row collapses to the
   name line). Never guess.

Parsing — **tail, do not full-parse**: large sessions can exceed 20 MB.
Read the last 256 KiB, split on `\n`, scan lines from the end for the first entry with
`type == "assistant"`, `message.usage` present, and `isSidechain != true`. If none is
found, grow the window ×4 up to a 16 MiB cap, then give up. Lines may be partial at the
window boundary; skip unparseable lines.

Formula (verified live on a real transcript: input 2 + cache_read 128,982 +
cache_creation 6,131 on `claude-opus-4-8`):

```
used   = usage.input_tokens
       + usage.cache_read_input_tokens (default 0)
       + usage.cache_creation_input_tokens (default 0)
window = context_window(model)          # see below
pct    = round(100 * used / window)     # clamp 0..100
token  = pct < warn  → ctx_ok   = "{pct}%"
         pct < hot   → ctx_warn = "{pct}%"
         else        → ctx_hot  = "{pct}%"
```

Defaults `warn = 50`, `hot = 80`, configurable. The other two `ctx_*` keys are cleared
in the same report so the row never shows two percentages.

`model` short form: `claude-opus-4-8` → `opus`, `claude-sonnet-*` → `sonnet`,
`claude-haiku-*` → `haiku`; unknown → the segment after `claude-` or the raw id
truncated to 8 chars. Sourced from the same assistant entry (`message.model`).

Context window: 200,000 for Opus/Sonnet/Haiku by default. The 1M-context variant is
**not** distinguishable from the model id in the transcript, so:

- `[context_window.by_model] "claude-opus-4-8" = 1000000` is a documented override.
- `auto_promote_1m = true` (default): if `used > default window`, the window cannot be
  200k; treat it as 1M for that pane. Self-correcting for the common case, wrong only
  while a 1M session is still below 200k (it will show a too-high % until it crosses).
  Stated as a known limitation; the override fixes it for users who always run 1M.

### 5.3 Login / account → `login`, `org` (default OFF)

Source: `~/.claude.json` → `oauthAccount.emailAddress`, `oauthAccount.organizationName`
(the keys Claude Code writes to that file).

Product decision: for a single-account user this is the same string on every row and
the governing rule says kill it. v1 ships the reader and the tokens behind
`[tokens] login = "off" | "always"`. The `"auto"` mode (show only when values differ
across rows) is reserved for phase 2, when per-pane `CLAUDE_CONFIG_DIR` detection
(via `herdr pane process-info` environment) makes it possible for rows to differ. In
v1 there is a single file, so `auto` would be indistinguishable from `off` and is not
offered, to avoid promising something that cannot trigger.

### 5.4 Future tokens (listed, not designed)

`ago` (last assistant timestamp → `3m`), `branch` as its own token, `cost` / `turns`
from the transcript, rate-limit headroom, and an attention-first `agent.view.set`
projection (`socket-api.mdx:413-450`).

---

## 6. QA loop

"Does this help?" can only be judged visually. The loop is part of P1's definition of
done, not an afterthought.

### 6.1 Recreate the real pain (scripted, `qa/scenario.sh`)

Using only the herdr CLI (`workspace create --cwd --label`, `agent start --kind
claude`, `pane report-agent --state blocked --source custom:qa`, all in
`cli-reference.mdx:118-124,231-238,301`):

- 4–5 workspaces all labelled `dashboard`, pointing at distinct worktrees/branches
  (`auth-mw`, `billing`, `main`, a second `main` in a differently named dir).
- 1 agent forced `blocked`.
- 1 agent at ~90% context, 1 on `haiku`, 1 with no transcript yet (fresh start).

Deterministic data without burning tokens: `AGENTS_INFO_FIXTURE_DIR` makes the plugin
read `<fixture>/<pane_id>.jsonl` instead of `~/.claude/projects/...`. The QA scenario
ships fixture transcripts for 12%, 44%, 91%, `haiku`, and "no usage yet".

### 6.2 Apply

`cargo build --release && herdr plugin link "$PWD"`, apply the recipe from 2.1, then
`herdr plugin log list --plugin rchougule.agents-info` to confirm the startup sweep ran.

### 6.3 Capture (`qa/shoot.sh`)

macOS `screencapture`: `screencapture -l "$(GetWindowID <TerminalApp> --list | ...)"`
for the whole terminal window, or `screencapture -R x,y,w,h qa/out/<variant>.png` for
the sidebar region. Shots are taken in: expanded and collapsed sidebar, grouped
(`agent_panel_sort = "spaces"`, the reference setting) and flat sort, dark and light
theme, and at sidebar widths 18 / 26 / 36.

### 6.4 Grade

A fresh reviewer (given only the screenshots and the checklist below) fills a
scorecard per shot. It must answer: can every row be told apart in one glance? does
the % pop at 91% and recede at 12%? is any distinguisher truncated into sameness?

### 6.5 Adjust, re-shoot, A/B

Tune caps, abbreviation table, thresholds, colors; re-shoot; repeat. A/B two layouts:

- **A (default above)**: line 2 = `agent · model · ctx`.
- **B**: line 2 = `model · ctx` (drop `agent`, redundant on a Claude-only layout;
  buys 9 columns for the name line at narrow widths).

### 6.6 Decide

Send the before/after pair and the A/B pair to the human; they pick the default
layout. Curated shots are committed under `docs/qa/` (the only PNGs not ignored).

### 6.7 QA checklist

- [ ] Truncation: with 5 × `dashboard`, every `name` prefix differs within 6 chars.
- [ ] Truncation: no `…` inside `name` at width 26 for branch names ≤ 14 chars.
- [ ] Meta line: line 2 indents 3 columns and aligns across rows.
- [ ] % color: 12% green, 44% green, 55% amber, 91% red; only one `ctx_*` per row.
- [ ] Theme: readable on catppuccin dark and on a light theme; amber legible on light.
- [ ] Grouped (`spaces`) and flat sort both render both lines.
- [ ] Collapsed sidebar still shows the state icon column; nothing overflows.
- [ ] Widths 18 / 26 / 36: the row degrades by dropping `model`, never `name`.
- [ ] Fallback: no transcript → one-line row, no stray ` · `.
- [ ] Fallback: unknown model id → short raw id, no panic.
- [ ] Fallback: 1M session past 200k → auto-promoted, plausible %.
- [ ] Fallback: agent just started, no assistant usage yet → no `ctx_*`, `model` absent.
- [ ] Non-Claude rows unchanged (Codex row still shows workspace/tab/agent).
- [ ] Restart herdr: sidebar repopulates within ~1s via the startup sweep.
- [ ] Close a pane: its siblings' names recompute (collision resolved).

---

## 7. Phasing

### P1 — Claude end-to-end (definition of done: QA loop run once, human picked a layout)

- Crate skeleton, `herdr-plugin.toml`, `sweep` / `enrich` modes, `HerdrClient` trait
  with CLI implementation.
- `name`/`sub` composer with collision resolution and caps.
- Transcript tail reader: `ctx_*`, `model`, context-window table + `auto_promote_1m`.
- Login reader wired but `off`.
- Fixtures, unit tests, fake-herdr integration test, snapshot tests, CI green.
- `qa/scenario.sh`, `qa/shoot.sh`, first before/after shots under `docs/qa/`.
- README with the recipe; `herdr plugin link` dev flow documented.

### P1.5 — polish from QA

- `ago` token, `branch` as a standalone token for users who want it on line 2.
- Abbreviation table and caps tuned from screenshots; A/B outcome applied as default.
- `login = "always"` verified visually; light-theme color pass.
- Optional `--ttl-ms` safety net (e.g. 12h) if stale tokens are ever observed.

### P2 — breadth

- Codex adapter (its own session/transcript layout), then Cursor. Adapter trait:
  `fn enrich(pane: &PaneFacts) -> Tokens` per agent kind; `rows_by_agent.codex` recipe.
- `cost` / `turns` tokens; rate-limit headroom if a local source exists.
- Attention-first `agent.view.set` projection as a plugin action.
- `login = "auto"` via per-pane `CLAUDE_CONFIG_DIR` detection.
- Optional live-% daemon (same enrichment code; trigger swapped to
  `events.subscribe` / transcript mtime watch). Only if P1 feedback asks for it.

---

## 8. Delivery

### 8.1 Git workflow

- Default branch `master`. Work on `feat/<topic>` branches (`feat/name-composer`,
  `feat/transcript-ctx`, `feat/qa-loop`); squash-merge to `master`.
- Conventional commits (`feat:`, `fix:`, `test:`, `docs:`, `chore:`, `ci:`). Every
  commit carries the session trailers used in this repo's initial commit.
- Tags `v0.1.0` etc. on `master`; the manifest `version` must match the tag.

### 8.2 Testing approach (TDD; tests land before the code they exercise)

- **Unit** — `name.rs` collision fixtures (the 5 × dashboard table in 5.1 is test #1),
  slug computation, model short-form table, threshold selection, tail-window growth,
  partial-line skipping, `isSidechain` filtering.
- **Fixture transcripts** under `tests/fixtures/transcripts/`: 12%, 44%, 91%, haiku,
  no-usage-yet, 1M-past-200k, and a >256 KiB file whose last usage sits just beyond
  the first window. Redacted, hand-trimmed real shapes.
- **Integration** — a fake `herdr` executable on `PATH` (shell shim that records argv
  to a file and replays canned `agent list` / `pane list` / `tab list` /
  `workspace list` JSON). Tests assert the **exact** `report-metadata` argv per pane,
  including `--clear-token` of the two inactive `ctx_*` keys and monotonic `--seq`.
  This mocks the boundary we actually call (the CLI via `HERDR_BIN_PATH`); a raw
  socket mock is only needed if the phase-2 daemon switches transport.
- **Snapshot** (`insta`) — the full token map for the QA scenario, so a change in
  abbreviation or cap shows up as a reviewed diff.
- **Event parsing** — fixtures for each `EventData` variant used, plus the
  `HERDR_PANE_ID` fallback.

### 8.3 CI (GitHub Actions, on push and PR)

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, on
macOS and Linux runners with the toolchain pinned in `rust-toolchain.toml` (herdr uses
1.96.1; match it). Release builds on tag.

### 8.4 Distribution

- GitHub repo `rchougule/herdr-agents-info`.
- Dev: `cargo build --release && herdr plugin link "$PWD"`. `plugin link` does not run
  build commands (`plugins.mdx:220-222`), so the README says to build first.
- Install path once public: `[[build]] command = ["cargo","build","--release"]` runs
  on `herdr plugin install` (`plugins.mdx:218-231`); we document `cargo` as a required
  tool. The manifest command points at `target/release/agents-info`.
- Marketplace later: add the `herdr-plugin` GitHub topic; the index refreshes every
  30 minutes (`plugins.mdx:362-374`).

---

## 9. Open questions and risks

1. **1M-context detection.** Not derivable from the transcript model id. Mitigation:
   `by_model` override + `auto_promote_1m`. Residual: a 1M session below 200k shows an
   inflated %. Watch for a `[1m]` suffix or a `context_window` field appearing in
   newer Claude Code transcripts and prefer it if present.
2. **Transcript size.** Tail-read with growth, 16 MiB cap; never read a whole 100 MB
   file on a focus event. Risk of a partial JSON line at the boundary is handled by
   skipping unparseable lines.
3. **cwd-slug edge cases.** Only `/` → `-` is confirmed. Paths containing `.`, `_`,
   spaces, or unicode need checking against real directories in dev; the fallback
   (newest `.jsonl` under any dir whose name matches after normalisation) plus
   `foreground_cwd` covers `cd` inside the session. If both miss, report nothing.
4. **Does `agent_session` carry the Claude session id?** Yes by construction
   (`agent_resume.rs:63-69` + the Claude hook), but only when herdr's Claude
   integration hook is installed and fired `SessionStart` (it is installed on the
   reference setup: `~/.claude/settings.json` calls `herdr-agent-state.sh session`).
   Panes without it fall back to newest-jsonl, which can pick the wrong session when
   two Claude sessions share a cwd. Surface this in `agents-info doctor`.
5. **Sidechain entries.** Subagent traffic may appear in the main transcript with
   `isSidechain: true`; we skip those. Confirm in dev that the main line's last
   assistant entry is what Claude Code's own context indicator uses.
6. **Event JSON shape.** `EventEnvelope { event, data }` is verified in source; the
   exact string for `event` is not documented. Parser reads `data.*` only.
7. **Hook fan-out.** Focus and status events on 12 panes are still low-volume, but a
   burst (restore of a 12-pane session) spawns ~12 short processes plus the sweep.
   Acceptable; if not, `enrich` can early-exit when the state dir shows a report for
   that pane within the last 250 ms.
8. **Custom token default style** is dim; users copying only part of the recipe get a
   dim name. The README recipe is the whole block for that reason.
9. **`rows_by_agent` and `agent` token on mixed fleets** — Codex rows keep the herdr
   default until P2; the sidebar will look inconsistent across agent kinds in the
   meantime. Stated in README.

---

## Appendix A — `herdr-plugin.toml` sketch (dev phase owns the real file)

```toml
id = "rchougule.agents-info"
name = "Agents Info"
version = "0.1.0"
min_herdr_version = "0.8.2"
description = "Glanceable Agents sidebar: contextual name, model, and context % per Claude pane"
platforms = ["macos", "linux"]

[[build]]
command = ["cargo", "build", "--release"]

[[startup]]
command = ["target/release/agents-info", "sweep"]

[[events]]
on = "pane.agent_status_changed"
command = ["target/release/agents-info", "enrich"]

[[events]]
on = "pane.agent_detected"
command = ["target/release/agents-info", "enrich"]

[[events]]
on = "pane.created"
command = ["target/release/agents-info", "enrich"]

[[events]]
on = "pane.focused"
command = ["target/release/agents-info", "enrich"]

[[events]]
on = "pane.closed"
command = ["target/release/agents-info", "enrich"]

[[actions]]
id = "sweep"
title = "Agents Info: refresh all rows"
contexts = ["workspace"]
command = ["target/release/agents-info", "sweep"]

[[actions]]
id = "doctor"
title = "Agents Info: diagnose (transcript mapping, hook status)"
contexts = ["workspace"]
command = ["target/release/agents-info", "doctor"]
```

Notes: `command` is argv, no shell (`plugins.mdx:119-121`); runtime cwd is the plugin
root (`plugins.mdx:253`), so the relative binary path resolves. Plugin ids may use
letters, digits, `.`, `:`, `_`, `-` (`plugins.mdx:105-106`); action ids may not
contain dots (`plugins.mdx:108-111`).

## Appendix B — plugin config sketch (`$HERDR_PLUGIN_CONFIG_DIR/config.toml`)

```toml
[tokens]
login = "off"        # off | always   ("auto" arrives in P2)
model = true

[name]
max_name = 14
max_sub  = 10

[thresholds]
warn = 50
hot  = 80

[context_window]
default = 200000
auto_promote_1m = true
[context_window.by_model]
# "claude-opus-4-8" = 1000000
```

## Appendix C — proposed crate layout

```
herdr-plugin.toml
Cargo.toml / Cargo.lock / rust-toolchain.toml
src/main.rs               clap: sweep | enrich | doctor
src/herdr.rs              HerdrClient trait; CliClient spawning $HERDR_BIN_PATH
src/model.rs              serde subsets of AgentInfo / PaneInfo / TabInfo / WorkspaceInfo / EventEnvelope
src/name.rs               composer + collision resolution + abbreviations + caps
src/claude/transcript.rs  tail reader, usage → pct, model short form
src/claude/window.rs      context-window table + auto-promote
src/claude/account.rs     ~/.claude.json reader
src/render.rs             facts → token map → ReportPlan (argv per pane)
src/config.rs             plugin config (Appendix B)
tests/fixtures/           transcripts, agent_list.json, events/*.json
tests/enrich_cli.rs       fake-herdr integration test asserting exact argv
tests/snapshots/          insta
qa/scenario.sh, qa/shoot.sh, docs/qa/*.png
.github/workflows/ci.yml
```
