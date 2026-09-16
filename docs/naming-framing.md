# Naming framing — which fields a Claude row shows, when, and how they look

Status: **canonical**. `src/name.rs`, `src/pack.rs` and `src/render.rs` implement this
document; when they disagree with it, the code is wrong. Supersedes PLAN.md §5.1 (the
role-swap design), the "pane slot = git branch" behaviour in `display_fields()`, the
shared `$m1/$m2/$m3` tokens, and the "name-only sibling re-report" in PLAN §4.2.

The job: with ~13 Claude agents, several sharing a workspace label, the user must pick
the right row in one glance. Every rule below exists to serve that and nothing else.

---

## 1. Principles (the whole framing in seven lines)

1. **Identity is shown, never replaced.** The bold workspace always leads. A
   distinguisher is *appended* after it; nothing ever takes its place.
2. **Human-typed beats derived.** A name the user set (`pane rename`, `agent start
   <name>`, a tab they named) always outranks anything we compute (branch, directory,
   pane id). A derived value never displaces a typed one.
3. **Derived values appear only when they earn it** — to split rows that would
   otherwise read the same, or to give a bare-workspace row *some* context. Never
   unconditionally. A field that reads the same on every row is noise; kill it.
4. **Kill redundancy.** A field equal to one already placed on the row is dropped.
5. **Order is fixed; presence is conditional.** The ladder order never changes. Each
   rung has a show/hide rule. The packer receives the rungs that survive, in order.
6. **A row is a pure function of a snapshot.** `row(p) = F(own(p), fleet_snapshot)`,
   deterministic. Which pane is focused, which event fired, and what was reported last
   time are **not inputs**. A row may change only when its own fields change or the
   set of identity keys in the fleet changes. (§5 spells out the contract.)
7. **Color means urgency; weight means hierarchy.** The only colored text on a row is
   the `ctx%`. Everything else is told apart by bold / normal / dim, which are
   theme-independent. (§7.)

---

## 2. Fields and normalisation

| field | source | role |
|---|---|---|
| `workspace_label` | `workspace list` | identity, rung 0 (herdr renders it bold; always present) |
| `tab_label` | `tab list` | identity, rung 1 |
| `pane_label` | `pane list` `.label` (user `pane rename`) | identity, rung 2 (highest intent) |
| `agent_name` | `agent list` `.name` (`agent start <name>`) | identity, rung 2 fallback |
| `git_branch` | `.git/HEAD` of `foreground_cwd ?? cwd` | derived: splitter / hint only |
| `cwd_basename` | basename of `foreground_cwd ?? cwd` | derived: splitter / hint only |
| `short_pane_id` | `p4T` from `w1M:p4T` | derived: last-resort splitter only |
| `model` | transcript (cached per pane, §5) | metadata |
| `ctx%` | transcript (cached per pane, §5) | metadata, anchored end of line 1 |
| `status`, `agent`, `login`, `org`, `terminal_title*` | — | **not shown** (see §10) |

**`norm(s)`** — applied before every comparison and before display:
trim; collapse internal whitespace runs to one space; strip a leading `refs/heads/`;
strip a trailing `.git`; for *comparison only*, Unicode-lowercase. Empty after
normalisation counts as absent. Display keeps original case.

**Equality** anywhere in this document means `norm(a) == norm(b)` (case-insensitive).
There is no prefix, fuzzy, or substring matching — `Vector Search` and
`vector-search-plan` are *different* on purpose; cleverness here is how we got
`master · 1`.

**Composite tab**: a `tab_label` containing the herdr token separator ` · ` (space,
middle dot, space). herdr and its plugins compose tab titles with this separator;
users do not type it. A composite tab is treated as **absent** (§3 rung 1).

**Default branch**: `norm(git_branch) ∈ {main, master}`. Used only to order the thin-row
hint (§6); default branches are still valid splitters (§4).

---

## 3. The identity ladder

The packer's ordered input list is built as follows. Rung 0 is not part of the list
(herdr renders it as the built-in `workspace` token); rungs 1–2 are the typed identity;
§4/§6 may append derived items after rung 2; §7 appends metadata.

| rung | item | class | show when | else |
|---|---|---|---|---|
| 0 | `workspace` | leader | always (herdr built-in, bold) | if somehow empty, use `cwd_basename`, then `short_pane_id` — a row must have a leader |
| 1 | `tab` = `tab_label` | typed | present, **not composite**, and ≠ `workspace` | omit |
| 2 | `pane` = `pane_label` ?? `agent_name` | typed | present and ≠ `workspace` and ≠ `tab` (the first of the two that is present is taken; if it is redundant, do **not** fall through to the other — one pane slot, one source) | omit |

Rung 2 is the highest-intent field on the row. It is **never** dropped for a derived
value, never truncated in favour of metadata, and never displaced by a splitter — a
splitter comes *after* it.

**Cryptic short tabs** (`1`, `2`, worktree indices) are identity like any other tab.
The user sees them in the tab bar and recognises them. No length heuristic.

**Thin row**: after applying rungs 1–2, the list is empty (nothing but the
workspace). Thin rows are the only rows eligible for the §6 hint.

---

## 4. Collision resolution (append a splitter, only on collision)

**Identity key** of a row = the tuple of *displayed* identity items after §3:
`(norm(workspace), norm(tab) or "", norm(pane) or "")`.

Group **all Claude panes in the snapshot** (across workspaces — same-label Spaces are
the whole point) by identity key. Any group of size ≥ 2 is a collision group and is
resolved as follows. Rows outside a collision group get nothing appended here.

```
resolve(group G, candidates = [git_branch, cwd_basename, short_pane_id]):
  for c in candidates:
    vals = { norm(c(row)) or ABSENT : row in G }
    if |distinct(vals)| == 1: continue           # c does not split G; adding it is noise
    for row in G:
      if c(row) present and c(row) ∉ already-shown items on row:
        append c(row) to row.derived
    partition G by (identity key + appended items)
    for each sub-group S with |S| ≥ 2:
      resolve(S, candidates after c)
    return
  # unreachable: short_pane_id is unique per pane, so the last candidate always splits
```

Rules the pseudocode encodes, stated plainly:

- A candidate is appended **to every member of the group being resolved**, so the group
  keeps one visual shape (`… · master` / `… · dist`), not one row with a suffix and one
  without.
- A candidate that is identical across the group (five worktrees all on `master`) is
  **skipped**, not appended. This is the fix for "every experiment row showed
  `master`".
- A row whose candidate is absent (not a git dir) appends nothing; its absence is the
  distinguisher.
- A candidate equal to something already on that row (branch `auth-mw` under tab
  `auth-mw`) is not appended to that row.
- Recursion only touches the sub-groups that *still* collide; rows already split are
  left alone. Terminates because pane ids are unique.
- Branch abbreviations still apply to appended branches: `feature/` → `f/`,
  `bugfix/` → `b/`, `release/` → `r/`, `origin/` dropped.

The splitter is the **only** sibling-dependent thing on a row, and it is allowed only
under the contract in §5.

---

## 5. The purity contract (pure per pane; ruling on sibling-dependent fields)

### 5.1 The rule

A row's displayed content is a pure, deterministic function of two inputs and nothing
else:

- **`own(p)`** — that pane's own fields: labels, cwd, branch, its cached `model`/`ctx`.
- **`fleet_snapshot`** — one `agent list` + label lists taken at the start of the
  invocation, reduced to the multiset of identity keys and splitter candidate values.

**Not inputs**: which pane is focused, which event kind triggered the invocation, which
pane the event named, what was reported previously, wall-clock time. Focusing or
clicking a sibling therefore cannot change a row, by construction — the function has
no argument through which that information could arrive.

### 5.2 Ruling on collision-dependent distinguishers

The question a splitter raises: does one that depends on the sibling set
reintroduce sibling coupling and this bug class? **Partly, and we accept it under
constraints — we do not forbid it.**

Forbidding it would leave the two `dashboard cache check` rows and the two
`experiment · 1` rows byte-identical — the headline job (disambiguation) failing on the
exact fleet that motivated the plugin. A purely per-pane approximation ("show branch
when the tab is short") is the heuristic we already rejected. So the splitter stays,
and the coupling is made *safe* rather than removed:

1. **One snapshot, whole fleet, every invocation.** `sweep` and `enrich` both fetch the
   full snapshot (enrich already does — PLAN §4.2) and compute `F` for **every** Claude
   pane. There is no "target pane" code path for identity and no "cheap sibling" path.
   The event only decides *which pane's transcript is re-read*, never how rows are
   computed.
2. **Full reports only.** A report to a pane always carries the plugin's **entire**
   token set for that pane: every key the plugin owns is either set or explicitly
   cleared in the same call. A partial report (a subset of keys) is a bug. There is one
   function `report(pane_id, RowTokens)` and `RowTokens` has no optional "skip this
   field" state.
3. **Metadata is cached per pane and never dropped by identity work.** `model` and
   `ctx%` come from a per-pane cache file (`$HERDR_PLUGIN_STATE_DIR/<pane_id>.json`)
   written on every transcript read. Computing a sibling's row reads the cache; it
   never re-reads a transcript and never emits the row without metadata. A cache miss
   means that pane has never had a transcript read, in which case clearing is a no-op.
4. **Metadata lives in its own tokens.** `model` is never packed into the same token as
   identity (§7). Even if rule 2 were violated, an identity rewrite could not remove
   the model. Belt and braces.
5. **Idempotent skip (event path only).** On an `enrich` (an event), if `F` yields the
   same `RowTokens` as the cache holds for that pane, no report is sent — a focus event on
   a stable fleet produces zero reports. **`sweep` never skips:** it re-pushes every pane
   unconditionally. Sweep is the startup / restart / "refresh all rows" path, where
   herdr's own display state has been reset but the persisted cache still holds the last
   tokens; skipping there would leave every row blank until an event happened to change a
   pane's tokens.
6. **Sibling rows may legitimately change only when the snapshot changes**: a Claude
   pane appears (`pane.created`, `pane.agent_detected`), disappears (`pane.closed`), or
   a label is renamed (covered by the sweep action and the next snapshot). This is
   intended — a collision dissolving should un-split the survivor.

Under these six rules a splitter is a function of the snapshot, not of the sibling's
*activity*, which is the property the pure-per-pane rule actually protects.

---

## 6. Thin-row context hint (append one hint, only when otherwise bare)

Applies **only** to a row that is thin (§3) **and** received no splitter in §4.
It satisfies the original ask, "if only the workspace shows, give me something", and
nothing more. Exactly one item is appended, the first available of:

1. `git_branch` if present, **not** a default branch, and ≠ `workspace`
2. `cwd_basename` if present and ≠ `workspace`
3. `git_branch` if present (a default branch) and ≠ `workspace`
4. nothing — a bare workspace is better than a pane id

Rationale for the order: a feature branch says what the agent is *for*; a directory that
differs from the Space name says *where* it is; `master` says almost nothing but is
still better than blank when it is all we have. `short_pane_id` is never a hint (§8g).
The hint is per-pane (no sibling input) and lands in the `derived` class.

Ordering of operations for one row: §3 → §4 → §6 → §7. §6 cannot fire on a row §4
touched, so a row never carries two derived fields unless §4 recursion needed both.

---

## 7. Visual encoding and token layout

### 7.1 Constraints

herdr tokens support **fg color, bold, dim** — nothing else (no italic, underline is
rejected; `src/config/sidebar.rs`). Style is static per config token; a token can carry
one style. Custom tokens default to dim. Fixed hex colors are the only theme-dependent
element: bold/dim adapt to any theme, a hex does not.

### 7.2 Ruling on the tension: option (c), tokens split by style class per line

Per-field color needs per-field tokens; the packer wants to pack freely into shared
tokens. Options considered:

- **(a) keep `$m1/$m2/$m3`, per-line color only.** Rejected. Tab and pane name — the
  disambiguators — stay the same flat dim as `opus`, which is a large part of why rows
  read alike today. It also keeps identity and model in one token, the live bug.
- **(b) one token per field on a fixed layout.** Rejected. `study · flow · opus 5%`
  becomes two lines and `Vector Search` three; herdr's overflow (drop flexible tokens
  front-first, re-enable from the end) would squeeze identity before metadata.
- **(c) hybrid — chosen.** Three style classes; **one token per class per line**; the
  packer flows items exactly as it does now, but emits, per line, the contiguous run
  of each class. Because the ladder order is `typed → derived → model` and items flow
  in ladder order, every line's items are already grouped by class in that order, so
  one token per class per line loses nothing. Compactness is fully preserved.

Token set (9 + 3 = 12 keys, all owned and always set-or-cleared per §5.2):

| class | tokens | contents |
|---|---|---|
| typed identity | `$t1 $t2 $t3` | tab, pane (rungs 1–2), joined with ` · ` |
| derived | `$d1 $d2 $d3` | splitters (§4) / hint (§6) |
| model | `$mo1 $mo2 $mo3` | `opus` etc., exactly one set (§7.4) |
| ctx | `$ctx_ok $ctx_warn $ctx_hot` | `NN%`, exactly one set |

Row template (`rows_by_agent.claude`):

```
line 1: state_icon · workspace(bold) · $t1 · $d1 · $mo1 · $ctx_ok|$ctx_warn|$ctx_hot
line 2:                                $t2 · $d2 · $mo2
line 3:                                $t3 · $d3 · $mo3
```

herdr draws ` · ` only between visible tokens and drops a line with no visible tokens,
so empty classes and empty lines cost nothing (PLAN §2.1).

### 7.3 Field → style

| field | class | style | why |
|---|---|---|---|
| state icon | herdr | herdr's | untouched (JTBD 2) |
| workspace | leader | **bold**, `dim = false`, no fg | the thing the user recognises first |
| tab, pane name | typed | normal: `dim = false`, not bold, no fg | the disambiguators must be *readable*, one step under the leader |
| branch / dir / pane id | derived | dim | computed, secondary; visibly "not something you typed" |
| model | model | dim | metadata; present on every row, must recede |
| ctx% < 50 | ctx | fg green | calm |
| ctx% 50–79 | ctx | fg amber | attention |
| ctx% ≥ 80 | ctx | fg red, **bold** | the one alarm on the row |

Hierarchy is bold > normal > dim and works identically on catppuccin mocha and on any
light theme. Color is spent on exactly one thing, urgency, so a glance that catches
color always means "context". No identity field ever gets a hue.

Default hexes (catppuccin mocha, the default theme): green `#a6e3a1`, amber `#f9e2af`,
red `#f38ba8`. The README ships a second block for light themes (catppuccin latte:
`#40a02b` / `#df8e1d` / `#d20f39`). Three hexes are the entire theme surface; QA (PLAN
§6.7) checks amber legibility on light.

### 7.4 Metadata placement

- **`ctx%`** — always anchored at the end of line 1, coloured by threshold. Absent when
  there is no transcript/usage yet (no reserve is held for it then).
- **`model`** — always shown when known (JTBD 4; fleets mix opus/fable/sonnet). Identity
  items are laid out first, in order, whole-per-line as the packer already does;
  `model` then fills the **earliest line with spare room**, including line 1 beside
  `ctx%`, and is reported in that line's `$moN`. Model is metadata, so it may sit above
  an identity item that overflowed — the one permitted reordering; it keeps rows at two
  lines instead of three.
- Identity is never truncated to make room for `model`. If `model` fits nowhere, it
  goes on the last line and is truncated there, never an identity item.
- No other metadata is rendered as text (§10).

---

## 8. Rulings on the hard cases

**(a) Two rows with identical workspace + tab** (the two `dashboard cache check`).
Yes, add a splitter — **only on collision, never always** (§4), under the §5 contract.
The candidate walk is branch → cwd basename → short pane id, skipping any candidate
that is the same on both rows. Both on `master` in the same directory → they get
`p4T` / `p9K`. The differing model (`opus` vs `fable`) does **not** count: model is
metadata, changes with `/model`, and often matches. Identity must stand on its own.

**(b) Only-workspace rows.** Not a collision → §6 hint: non-default branch, else
directory ≠ workspace, else default branch, else nothing. A collision *among* thin rows
is handled by §4 first and gets no additional hint.

**(c) Pane the user named vs one they did not.** Named: `pane_label` is rung 2, shown
after the tab in the typed class (normal weight), never displaced. Unnamed: the pane
slot is simply **empty**. Nothing derived fills it; a derived value can appear only via
§4/§6, in the dim derived class, *after* where the pane would be. This is the bug fix
for `backend-cache` vanishing behind `master`.

**(d) Cryptic short tabs `1` / `2`.** Identity, shown as-is (§3). No length heuristic.
The `experiment` pairs that share a tab are a §4 collision and get a splitter; the
pair that does not collide gets nothing.

**(e) The long plugin-generated tab** (`claude · 4 comments · 4 on s…`). **Drop it**
from the ladder (composite tab, §2). It is status text, not identity: it contains the
herdr separator, its first segment is the agent kind (redundant on a Claude row), and
its remaining segments are counters that change over time — showing it would also
violate §5 in spirit, since the row would change with activity. The row then becomes
thin and receives a §6 hint — `core · <branch|dir>` — which is more useful than
`claude · 4 comments ·…` mush. Non-composite tabs longer than a line are **truncated
by the packer**, never dropped or shortened by us (the tail-cut is honest; a shortening
rule would be another heuristic to re-decide later).

**(f) Should `git_branch` ever appear when a pane name exists?** Only as a §4 splitter,
i.e. only when two rows have the *same* workspace, tab **and** pane name — practically
never. Otherwise **no**: the pane name is the user's own disambiguation and the branch
adds nothing to it.

**(g) Should `short_pane_id` ever be shown?** Only as the **last** §4 splitter, when
branch and directory both fail to split a group. Never as a thin-row hint — empty is
better than `p4T` on a lone row. It is meaningless on first read but stable for the
pane's lifetime, so the user can learn it, and two identical rows are strictly worse.
Its appearance is also the nudge to `pane rename`, which fixes the row permanently.

**(h) A sibling is focused / clicked.** Nothing on any row changes, and no report is
sent (§5.2 rules 1, 5). If a row *does* change on focus, that is a purity violation
and a P0 bug, not a tuning question.

---

## 9. The real fleet under this framing

Assumed budgets: 24 columns on line 1 (before the ` · ` herdr draws after the
workspace), 22 on lines 2–3. `%` is the coloured ctx token. Exact column arithmetic is
the packer's; this table shows the *fields, their class, and their order*, and an
indicative render. Style legend in the render: `**bold**` leader, plain = typed
(normal), `_dim_` = derived and model, `%` colored. Assumptions where the input was
ambiguous are marked †.

| workspace | inputs | typed → derived → model | indicative render |
|---|---|---|---|
| `study` | tab `flow`, main, opus, 5% | `[flow] [] [opus]` | `**study** · flow · _opus_  5%` |
| `Vector Search` | tab `vector-search-plan`, opus, 73% | `[vector-search-plan] [] [opus]` | `**Vector Search** · _opus_ 73%` / `vector-search-plan` (model floats up, §7.4) |
| `dashboard slow` A | tab `dashboard cache check`, master, opus | **collision with B** → branch same, dir same† → `[dashboard cache check] [p4T] [opus]` | `**dashboard slow**  42%` / `dashboard cache check` / `_p4T · opus_` |
| `dashboard slow` B | tab `dashboard cache check`, master, fable | `[dashboard cache check] [p9K] [fable]` | `**dashboard slow**  18%` / `dashboard cache check` / `_p9K · fable_` |
| `dashboard slow` C | tab `k8s`, master, opus | `[k8s] [] [opus]` | `**dashboard slow** · k8s 12%` / `_opus_` |
| `dashboard slow` D | tab `last enriched at fixes`, dist, fable | `[last enriched at fixes] [] [fable]` | `**dashboard slow**  61%` / `last enriched at fixes` / `_fable_` |
| `dashboard slow` E | tab `postgres analytics`, pane `backend-cache`†, master, opus | `[postgres analytics, backend-cache] [] [opus]` | `**dashboard slow**  30%` / `postgres analytics` / `backend-cache · _opus_` |
| `lci fast` | tab `remaining lci elephants`, no transcript | `[remaining lci elephants] [] []` | `**lci fast**` / `remaining lci elephant…` (23 > 22; honest tail-cut) |
| `core` | tab `claude · 4 comments · 4 on s…` (composite → dropped), master, dir `core`† | thin → hint rule 3 → `[] [master] [opus]` | `**core** · _master · opus_ 40%` |
| `experiment` E1 | tab `1`, master, opus | **collision with E2** → branch splits → `[1] [master] [opus]` | `**experiment** · 1 · _master_ 22%` / `_opus_` |
| `experiment` E2 | tab `1`, dist, fable | `[1] [dist] [fable]` | `**experiment** · 1 · _dist_ 15%` / `_fable_` |
| `experiment` E3 | tab `2`, master, opus | no collision (E4's key includes its pane name) → `[2] [] [opus]` | `**experiment** · 2 · _opus_  9%` |
| `experiment` E4 | tab `2`, pane `herdr-agent-info-plugin`†, master, fable | `[2, herdr-agent-info-plugin] [] [fable]` | `**experiment** · 2 33%` / `herdr-agent-info-plugi…` / `_fable_` |

† A: assumed both `dashboard cache check` panes share a worktree; if their directories
differ, the splitter is the directory basename instead of the pane id. E/E4: the task
did not say which pane carries the label; placed on the most plausible row. `core`:
branch and directory not given; shown as the default-branch fallback.

Eyeball check against JTBD 1: every row is distinct at a glance, every workspace is
still the leader, `backend-cache` and `herdr-agent-info-plugin` are visible at normal
weight, `master` appears only where it splits (E1/E2) or where a row would otherwise be
bare (`core`) and is dim there. Focusing any of these rows changes none of them. The
one wart is E4's 23-column pane name losing its last character — user text hitting
the assumed width; QA item, not a framing change.

---

## 10. What changed from the proposed ladder, and why

**Kept from the proposal**: rung order `tab → pane → (derived) → model`; pane =
`pane_label ?? agent_name`; the "thin only" gate on the context hint; append-never-
replace; `ctx%` anchored on line 1.

**Changed / added**

1. **A collision rule exists now (§4).** The proposal had none; two
   `dashboard cache check` rows differed only by an accidental model mismatch. The
   splitter walk (branch → dir → pane id) appends to the whole group and *skips*
   candidates that do not split it, which is what makes it legal to show `master` on
   collision without re-creating the "master everywhere" noise.
2. **A purity contract governs the one sibling-dependent field (§5).** Prompted by the
   live bug where `opus` toggled onto whichever row was last focused. Rows are
   `F(own, snapshot)`; whole-fleet recompute per invocation; full reports only;
   per-pane metadata cache; idempotent skip; model in its own tokens. The name-only
   sibling re-report from PLAN §4.2 is deleted.
3. **Thin-row hint is ordered by information, not by field (§6).** The proposal said
   `git_branch → cwd_basename`; that puts `master` ahead of a directory name that
   actually says something. Now: non-default branch → directory ≠ workspace → default
   branch → nothing.
4. **Composite tabs are dropped (§2, §8e).** The proposal treated every tab as identity;
   `claude · 4 comments · 4 on s…` is status and would have consumed a whole line as
   mush. Single detection rule: contains ` · `.
5. **Tokens are split by style class per line (§7.2).** `$m1/$m2/$m3` become
   `$t/$d/$mo × 3`. Same packing, same compactness; typed identity reads at normal
   weight, derived and model recede, and identity/metadata can no longer clobber each
   other.
6. **Color is urgency-only; hierarchy is bold/dim (§7.3).** Theme-independent by
   construction; the three `%` hexes are the whole theme surface.
7. **Model floats to the earliest line with room (§7.4).** Otherwise `Vector Search` is
   a three-line row for two short facts. Identity order is untouched; only metadata
   moves.
8. **Rung 2 does not fall through** when the first present source is redundant
   (`pane_label` equal to the tab does not cause `agent_name` to appear). One slot, one
   source, predictable.
9. **§4 runs before §6**, and §6 never fires on a row §4 touched. Prevents
   `dashboard · master · dash-v2`.
10. **Per-field caps (`name ≤ 14`, `sub ≤ 10`) are gone.** They belonged to the
    two-token design. The packer's whole-item-per-line flow plus last-resort tail-cut
    is the only truncation.

**START using**
- `pane_label` as first-class identity. It is already fetched; `display_fields()` just
  never reads it.
- The composite-tab test.
- Fleet-wide collision grouping on the *displayed* identity tuple, from one snapshot.
- A per-pane metadata cache (`model`, `ctx`, last `RowTokens`) in the plugin state dir.
- Class-per-line tokens `$t1..3 $d1..3 $mo1..3`.

**STOP using**
- `git_branch` in the pane slot — the root cause of `backend-cache` disappearing.
- Unconditional branch. The branch has exactly two entry points now: §4 and §6.
- The collision role-swap (PLAN §5.1 step 2). Rejected by the user; identity leads.
- `short_pane_id` as a general fallback. §4 last resort only.
- The "name-only" sibling re-report and any partial `report-metadata` call.
- Shared `$m1/$m2/$m3` tokens.
- D4 `"<agent> · <status>"` as a name candidate. Status is volatile (a name that
  flickers idle/working is unusable as identity and violates §5 in spirit) and herdr's
  state icon already carries it. **`status` is not surfaced as text.**
- `agent` text token on Claude rows (`rows_by_agent` is already Claude-only; it read
  `claude` on every row).

**Confirmed, with a note**
- `agent_name` *is* pane identity: it is typed by the user at `agent start`, one agent
  per pane, and costs nothing to honour. It ranks below `pane_label` because a rename is
  the later, more deliberate act. Nobody in the current fleet uses it; keep it, do not
  design around it.
- `login`/`org` stay default-off (PLAN §5.3) — identical on every row.
- `terminal_title_stripped` is not identity (Claude rewrites it per task, so it would
  also change with activity). It is a candidate for a future *metadata* token alongside
  `ago`, not for the ladder.

---

## 11. Acceptance fixtures the code must pass (names for `name.rs` / `pack.rs` / `render.rs` tests)

Ladder and redundancy
1. `pane_label_survives_branch` — pane `backend-cache`, branch `master`, tab present →
   typed contains `backend-cache`; derived empty.
2. `tab_equal_workspace_dropped` — tab == workspace (case/whitespace differing) → no tab.
3. `composite_tab_dropped_then_hint` — tab `claude · 4 comments`, no pane → thin → hint.
4. `pane_slot_does_not_fall_through` — `pane_label` == tab, `agent_name` present →
   neither shown.

Collision
5. `collision_branch_splits_all_members` — two rows same key, branches `master`/`dist`
   → both get their branch in derived.
6. `collision_same_branch_skipped` — two rows same key, both `master`, dirs differ →
   no branch on either; both get their dir basename.
7. `collision_falls_to_pane_id` — same key, same branch, same dir → both get pane id.
8. `collision_recurses_on_subgroup` — three rows, branches `master`/`master`/`dist` →
   `dist` row gets only `dist`; the two `master` rows get `master` **and** the next
   splitter.
9. `no_collision_no_splitter` — unique key, branch present → derived empty.
10. `splitter_not_duplicated` — tab `auth-mw`, branch `auth-mw`, collision → branch
    skipped on that row.

Thin hint
11. `thin_hint_prefers_feature_branch` — no tab/pane, branch `auth-mw` → `auth-mw`.
12. `thin_hint_default_branch_yields_to_dir` — branch `main`, dir `dash-v2` ≠ workspace
    → `dash-v2`.
13. `thin_hint_default_branch_last` — branch `main`, dir == workspace → `main`.
14. `thin_hint_never_pane_id` — no branch, dir == workspace → derived empty.

Purity (§5)
15. `focus_event_is_not_an_input` — same snapshot, enrich invoked for pane X then for
    pane Y → identical `RowTokens` for every pane both times.
16. `sibling_recompute_keeps_model` — pane Y has cached `model=opus`; enrich for pane X
    (a collision sibling) → Y's report still carries `$moN=opus` and one `$ctx_*`.
17. `report_is_always_full` — every `report-metadata` argv sets or clears all 12 keys.
18. `identical_output_skips_report` — snapshot unchanged since last run → zero reports.
19. `closing_sibling_unsplits_survivor` — two-row collision, one closes → survivor's
    derived is cleared on the next snapshot.

Packing and encoding (§7)
20. `model_floats_to_line1` — `Vector Search` case renders `$mo1=opus`, `$t2` = tab.
21. `classes_never_share_a_token` — for every fixture, no `$tN` contains a derived or
    model item and no `$dN`/`$moN` contains typed text.
22. `line_class_order_is_typed_derived_model` — items on any line appear in ladder
    class order.
23. The §9 table as an `insta` snapshot of the full fleet, all 12 tokens per pane.
