# Layout design — which field sits where on a Claude row

Status: **canonical for layout**. Supersedes `naming-framing.md` §7.2 (token set and row
template), §7.4 (metadata placement, including the model float), the phrase "anchored
end of line 1" in the §2 field table, and fixtures 20/22 in §11. Everything else in
`naming-framing.md` — which fields exist, the ladder, collision resolution, the purity
contract, the thin-row hint, styles (§7.3) — is unchanged and is *not* re-decided here.

The problem this fixes, in the user's words:

> things are not consistent. in dashboard slow k8s, that's tab name. then above that, it
> has model name at the place of k8s placeholder. it's hard to read now instead of making
> it easy. maybe fixed space separators would also help.

Root cause: the current packer flows identity greedily and lets `model` float to the
earliest line with room (§7.4). The same visual slot therefore holds a **tab** on one
row and a **model** on the next:

```
dashboard slow · opus · 12%      slot 2 = model
  dashboard cache check
dashboard slow · k8s · 12%       slot 2 = tab      ← the eye cannot lock a column
  opus
```

Priority for this redesign, set by the user: **consistency / scannability >
compactness.** A position must always mean the same kind of thing, even when that costs
a line.

---

## 0. What herdr lets us do, and the counting convention used below

Constraints (verified; nothing below assumes anything else):

- One static line template per agent kind (`rows_by_agent.claude`), up to 3 lines,
  applied to **every** Claude row. The plugin can leave tokens empty; herdr drops empty
  tokens and drops a line whose tokens are all empty.
- Tokens render left-to-right joined by ` · `. **No column alignment, no right-align,
  no padding** — trailing whitespace is trimmed. Consistency therefore has to come from
  a fixed field→line/slot assignment, not from pixel columns.
- Per-token style: `bold`, `dim`, one `fg` hex. Nothing else.
- `row_gap` = number of blank lines between agent entries; configurable.
- Lines 2–3 are indented under the state icon, which is why they have ~2 fewer usable
  columns than line 1.

Budgets, using exactly the packer's arithmetic (`src/pack.rs`): line 1 has
`24 − width(workspace) − (width("NN%") + 1)` columns for the items between the
workspace and the `%`, items joined at 3 columns each; lines 2–3 have 22. So on line 1,
`dashboard slow` (14) with `12%` leaves **6** columns — enough for `opus` (4) or `fable`
(5), not for `opus · k8s` (11). The mocks below were checked against these numbers.
Where an item overflows its line I show the honest tail-cut (`…`) the packer would emit.

Mock legend (applies to every mock):

```
●            herdr's state icon (line 1 only; lines 2–3 are indented under it)
workspace    the first token on line 1 is always the workspace, BOLD
NN%          the ctx token — the ONLY coloured text; g/a/r in the gutter = green/amber/red(bold)
opus fable   model — dim
~text~       derived item (branch / dir / pane id / hint) — dim; the ~ are not rendered
plain        tab / pane name — normal weight
│ …          gutter: notes, not rendered
```

The fleet used in every candidate (14 rows, the real one):

| # | workspace | tab | pane | derived | model | ctx |
|---|---|---|---|---|---|---|
| 1 | `study` | `flow` | | | opus | 5% |
| 2 | `Vector Search` | `1` | `vector-search-plan` | | opus | 82% |
| 3 | `dashboard slow` | `dashboard cache check` | | | opus | 12% |
| 4 | `dashboard slow` | `dashboard cache check` | `backend-caching-plan` | | fable | 18% |
| 5 | `dashboard slow` | `k8s` | | | opus | 12% |
| 6 | `dashboard slow` | `last enriched at fixes` | | | opus | 34% |
| 7 | `dashboard slow` | `postgres analytics` | | | fable | 29% |
| 8 | `lci fast` | `remaining lci elephants` | | | — | — |
| 9 | `core` | (composite, dropped) | | hint `master` | opus | 8% |
| 10 | `experiment` | `1` | | | fable | 77% |
| 11 | `experiment` | `1` | `herdr-agent-info-plugin` | | opus | 42% |
| 12 | `experiment` | `2` | | splitter `p4V` | opus | 5% |
| 13 | `experiment` | `2` | | splitter `p4W` | opus | 4% |
| 14 | `core` | `1` | `worktree-fix-desktop-…` | | opus | 26% |

Rows 3/4 and 10/11 are **not** collisions (their identity keys differ by the pane), so
they carry no splitter — as the framing says (§4).

---

## 1. Candidates

### A — Metadata header, identity block (one rung per line) — **recommended**

Rule: **line 1 is metadata, lines 2–3 are identity. Nothing ever crosses.**

```
line 1:  ● workspace · NN% · model          who · how full · which model
line 2:    tab  [· derived]                 rung 1  (thin row: the hint sits here, dim)
line 3:    pane [· derived]                 rung 2
```

- Model **never** floats. It has one home: line 1, after the `%`.
- Tab **never** appears on line 1, even when it would fit. Pane **never** appears on
  line 2, even when it would fit. One rung, one line.
- A derived item sits at the end of the line of the identity item it qualifies: after
  the pane if there is one, else after the tab; on a thin row it is the whole of line 2.
- Absent things vanish without shifting anything that remains, because every optional
  token is *after* every mandatory one on its line.

Full fleet:

```
● study · 5% · opus                        │ g
    flow                                   │
● Vector Search · 82% · opus               │ r
    1                                      │
    vector-search-plan                     │
● dashboard slow · 12% · opus              │ g
    dashboard cache check                  │
● dashboard slow · 18% · fable             │ g
    dashboard cache check                  │
    backend-caching-plan                   │
● dashboard slow · 12% · opus              │ g
    k8s                                    │
● dashboard slow · 34% · opus              │ g
    last enriched at fixes                 │  22 cols, fits exactly
● dashboard slow · 29% · fable             │ g
    postgres analytics                     │
● lci fast                                 │  no transcript → no %, no model; slots simply empty
    remaining lci elephant…                │  23 > 22, honest tail-cut (same as today)
● core · 8% · opus                         │ g
    ~master~                               │  thin row → §6 hint, dim, in the tab slot
● experiment · 77% · fable                 │ a
    1                                      │
● experiment · 42% · opus                  │ g
    1                                      │
    herdr-agent-info-plugi…                │  23 > 22, tail-cut
● experiment · 5% · opus                   │ g
    2 · ~p4V~                              │  §4 splitter follows the tab
● experiment · 4% · opus                   │ g
    2 · ~p4W~                              │
● core · 26% · opus                        │ g
    1                                      │
    worktree-fix-desktop-…                 │  22 cols, fits
```

Scorecard

- **Every slot means one thing?** Yes. Line 1: bold = workspace, colour = `%`, dim =
  model. Line 2: normal text = tab. Line 3: normal text = pane. Dim text on lines 2–3 =
  derived. A reader who has seen two rows knows where to look on all fourteen.
- **`%` scannable down the column?** As good as herdr allows. There is no right-align, so
  the `%` x-position jitters with the workspace length (5–14 chars). But it is always on
  the *first* line of an entry, always the *first* thing after the bold leader, and the
  *only* coloured text anywhere — so the scan is "bold word → colour", entry after entry.
  Putting `%` before the model removes the second source of jitter (`opus`/`fable` differ
  in width) and reads in importance order (who → how full → which model, matching the
  JTBD ranking). See §1.4 for the alternative order.
- **Lines per row:** 2 for rows without a pane name, 3 with one; 1 for a bare row with
  no transcript and no identity. This fleet: 32 lines (today's floating layout: ~26).
  Six lines is the price of the guarantee.
- **Fits at the assumed width?** Line 1 fits every workspace in the fleet — worst case
  `dashboard slow · 18% · fable` uses 5 of its 6 spare columns. It stops fitting at a 16-char
  workspace with `fable` (17 with `opus`); §3 says what happens then (the *model* is
  tail-cut, never the `%`, never an identity item).

### B — Identity-led (`workspace · tab` on line 1) — rejected

Rule as posed: line 1 = `workspace · tab`; line 2 = `pane · derived · model · %`; line 3
= overflow.

```
● study · flow                             │
    opus · 5%                              │ g   % on line 2
● Vector Search · 1                        │
    vector-search-plan                     │     "vector-search-plan · opus · 82%" = 31 > 22 → wraps
    opus · 82%                             │ r   % on line 3
● dashboard slow                           │     "dashboard slow · dashboard cache check" = 37 > 24 → tab wraps
    dashboard cache check                  │     tab on line 2 (row 5 has it on line 1) ← the original complaint, with tab instead of model
    opus · 12%                             │ g   % on line 3
● dashboard slow                           │
    dashboard cache check                  │
    backend-caching-plan                   │     line 3 is the last line: "fable · 18%" has NOWHERE to go — metadata lost
● dashboard slow · k8s                     │
    opus · 12%                             │ g   % on line 2
● dashboard slow                           │
    last enriched at fixes                 │
    opus · 34%                             │ g
● dashboard slow                           │     "dashboard slow · postgres analytics" = 35 > 24 → wraps
    postgres analytics                     │
    fable · 29%                            │ g
● lci fast                                 │     "lci fast · remaining lci elephants" = 34 > 24 → wraps
    remaining lci elephant…                │
● core                                     │
    ~master~ · opus · 8%                   │ g
● experiment · 1                           │
    fable · 77%                            │ a
● experiment · 1                           │
    herdr-agent-info-plugi…                │
    opus · 42%                             │ g
● experiment · 2                           │
    ~p4V~ · opus · 5%                      │ g
● experiment · 2                           │
    ~p4W~ · opus · 4%                      │ g
● core · 1                                 │
    worktree-fix-desktop-…                 │
    opus · 26%                             │ g
```

Scorecard

- **Every slot means one thing?** No. The tab is on line 1 in 8 rows and on line 2 in 5
  — the exact defect being fixed, moved from `model` to `tab`. Line 2 slot 1 is a tab in
  5 rows, a pane in 3, a model in 3, a derived item in 3.
- **`%` scannable?** Worst of the three: on line 2 in 6 rows and line 3 in 6 (and lost
  in one), at different x-positions, sometimes preceded by dim text and sometimes by
  normal text.
  The eye has to *search* each entry for the colour instead of finding it in a fixed
  place.
- **Lines per row:** 2–3, 35 total — *more* than A — and row 4 needs a 4th line that
  does not exist, so its model and `%` are silently lost. A layout that drops the
  urgency signal on the fullest-identity row fails the JTBD outright.

Leading with identity feels natural for a single row but is exactly wrong for a *list*:
identity items have wildly variable length (1–23 chars) and shove everything after them
around. Fixed-width things (`%`, model) must sit **before** the variable-length block,
not after it.

### C — A's slots, but identity flows (tab · pane share a line when they fit) + `row_gap = 1`

Rule: line 1 as in A. Lines 2–3: identity items flow whole-item in ladder order (today's
`flow()`), so `1 · vector-search-plan` shares line 2. Because entries then vary between
2 and 3 lines with less internal structure, add `row_gap = 1` to keep entry boundaries
visible.

Full fleet (blank lines are the `row_gap`; `←` marks where C differs from A):

```
● study · 5% · opus                        │ g
    flow                                   │

● Vector Search · 82% · opus               │ r
    1 · vector-search-plan                 │ ← 1 + 3 + 18 = 22, fits → 2 lines instead of A's 3

● dashboard slow · 12% · opus              │ g
    dashboard cache check                  │

● dashboard slow · 18% · fable             │ g
    dashboard cache check                  │   20 + 3 + 20 = 43 → pane wraps
    backend-caching-plan                   │ ← pane is line 3 slot 1 here, line 2 slot 2 in row 2

● dashboard slow · 12% · opus              │ g
    k8s                                    │

● dashboard slow · 34% · opus              │ g
    last enriched at fixes                 │

● dashboard slow · 29% · fable             │ g
    postgres analytics                     │

● lci fast                                 │
    remaining lci elephant…                │

● core · 8% · opus                         │ g
    ~master~                               │

● experiment · 77% · fable                 │ a
    1                                      │

● experiment · 42% · opus                  │ g
    1                                      │   1 + 3 + 23 = 27 > 22 → pane wraps; tab alone
    herdr-agent-info-plugi…                │ ← same shape as A, but only because it did not fit

● experiment · 5% · opus                   │ g
    2 · ~p4V~                              │

● experiment · 4% · opus                   │ g
    2 · ~p4W~                              │

● core · 26% · opus                        │ g
    1                                      │   1 + 3 + 22 = 26 > 22 → pane wraps
    worktree-fix-desktop-…                 │ ←
```

Scorecard

- **Every slot means one thing?** Line 1 yes (same as A). Lines 2–3 **no**: the pane
  name is line 2 slot 2 in row 2 but line 3 slot 1 in rows 4, 11, 14. It is a smaller
  inconsistency than today's (both positions are *after* the tab, both are normal
  weight, so a pane can never be mistaken for a model), but "the pane is wherever it
  fit" is the same species of rule the user rejected.
- **`%` scannable?** Same as A.
- **Lines per row:** identity flow saves exactly **one** line in this fleet (row 2: 31
  vs A's 32). `row_gap = 1` then adds 13 blank lines → 44. The gap costs thirteen times
  what the flow saved. Without the gap, C is A with one line saved and one guarantee
  lost.
- **"One line when possible, but only if consistent."** Explored and found empty: the
  only rule that is both one-line and consistent is fleet-global ("every row is one line
  iff *every* row's identity fits beside its metadata"), which never holds for a
  13-agent fleet with tabs like `dashboard cache check`. A per-row one-line rule is, by
  definition, the float. There is no consistent one-line layout; A's two lines are the
  floor.

### 1.4 Order on line 1: `% · model` vs `model · %`

Both are fixed-slot and both are consistent. The choice is scannability and degradation.

```
● study · 5% · opus                 ● study · opus · 5%
● Vector Search · 82% · opus        ● Vector Search · opus · 82%
● dashboard slow · 12% · opus       ● dashboard slow · opus · 12%
● dashboard slow · 18% · fable      ● dashboard slow · fable · 18%
● core · 8% · opus                  ● core · opus · 8%
```

`% · model` (left) is chosen:

1. The coloured token is adjacent to the bold leader, so identity and urgency are read
   in one saccade; the dim model trails and recedes, which is what dim is for.
2. One source of jitter instead of two: the `%` x-position depends only on the workspace
   width, not on `opus` vs `fable` vs `sonnet`.
3. Reading order equals the JTBD order: who → how full → which model.
4. Graceful overflow: when a workspace is too long for the line, the thing at the end —
   the least important token — is what gets cut (§3 makes the plugin do this cut, so
   herdr never tail-cuts the row itself).

The right-hand order has the "number at end of line" convention going for it and
nothing else; with no right-align there is no ragged-right column to gain from it.

---

## 2. Recommendation: **A**, with `% · model` on line 1 and `row_gap = 0`

Reasoning, tied to the jobs:

- **JTBD 1 — pick the right row in one glance.** In A, the disambiguators (tab, pane)
  own lines 2–3 outright, at normal weight, never sharing a line with metadata, never
  moving. Row 3 vs 4 differ by the presence of a line 3; rows 10 vs 11 likewise; rows
  12 vs 13 by the dim splitter at the end of line 2. Identity is a *block* the eye can
  read top-down, not a set of fragments scattered around a `%`.
- **JTBD 2 — context and model.** Every entry's first line is `bold · colour · dim`. The
  scan for "who is running hot" is a scan of first lines only, and the colour is always
  the first thing after the name. Model is always the last token of the first line.
- **The user's consistency priority.** A is the only candidate in which every
  (line, slot) pair has exactly one meaning across the fleet. It buys that with six extra
  lines over today (32 vs ~26) and one extra line over C. The user explicitly accepted
  that trade.
- **`row_gap = 0`.** A's template makes every entry self-delimiting: line 1 is the only
  line with a state icon and bold text at the left margin, and lines 2–3 are indented
  and never bold. A blank line would repeat information the layout already carries, at
  a cost of 13 lines on this fleet. Keep the config knob documented for users with tall
  sidebars who prefer air; do not default to it.
- **"Fixed space separators"** (the user's suggestion) are impossible as columns — herdr
  trims trailing whitespace and pads nothing — but A delivers what the suggestion was
  reaching for: a fixed *structural* separator. Line 1 / line 2 / line 3 is the column
  system, vertically.

Why not B: it re-creates the floating-slot defect with the tab, scatters the `%` over
lines 2–3, uses more lines than A, and loses metadata on the row with the most identity.

Why not C: it trades the pane's fixed position for one saved line, then spends thirteen
lines on a gap to compensate for the structure it gave up.

---

## 3. Implementation recipe for A

### 3.1 Tokens (fixed names; no class-per-line indices)

Since each field now has exactly one home, the tokens are named for the field. Eight
keys, every one set-or-cleared on every report (§5.2 rule 2 unchanged).

| token | content | line · slot | style |
|---|---|---|---|
| `workspace` | herdr built-in | 1 · 1 | bold |
| `$ctx_ok` / `$ctx_warn` / `$ctx_hot` | `NN%`, exactly one set | 1 · 2 | fg green / amber / red+bold (unchanged) |
| `$model` | `opus` etc. | 1 · 3 | dim |
| `$tab` | rung 1 (tab), tail-cut to 22 | 2 · 1 | normal |
| `$d2` | derived items when the row has **no** pane (splitters joined by ` · `, or the §6 hint) | 2 · 2 (whole line on a thin row) | dim |
| `$pane` | rung 2 (pane label ?? agent name), tail-cut to 22 | 3 · 1 | normal |
| `$d3` | derived items when the row **has** a pane | 3 · 2 | dim |

The old `$t1-3` / `$d1` / `$mo1-3` keys are retired. On first run after upgrade the
plugin clears them once per pane so nothing stale can render if a user still has the
old config block.

### 3.2 `rows_by_agent.claude` (drop-in for `~/.config/herdr/config.toml`)

```toml
[ui.sidebar.agents.rows_by_agent]
claude = [
  ["state_icon",
    { token = "workspace",  bold = true },
    { token = "$ctx_ok",    fg = "#a6e3a1" },
    { token = "$ctx_warn",  fg = "#f9e2af" },
    { token = "$ctx_hot",   fg = "#f38ba8", bold = true },
    { token = "$model",     dim = true }],
  [{ token = "$tab",  dim = false }, { token = "$d2", dim = true }],
  [{ token = "$pane", dim = false }, { token = "$d3", dim = true }],
]
```

Light theme (catppuccin latte): swap the three `fg` hexes for `#40a02b` / `#df8e1d` /
`#d20f39`, as the README already documents. No other theme surface.

`row_gap`: leave at herdr's default of `0`. Document `row_gap = 1` under the same
`[ui.sidebar.agents]` table as an opt-in for tall sidebars (verify the exact parent
table against herdr's `src/config/sidebar.rs` when writing the README; this repo does
not document it today).

### 3.3 Plugin assignment rule (replaces `flow()` + `place_model()` for layout)

Inputs are unchanged: `typed = [tab?, pane?]`, `derived = [...]`, `model?`, `pct?`,
from §3/§4/§6. Assignment is a fixed mapping, not a packer:

```
$ctx_*  = pct                    (one of three, by threshold; others cleared)
$model  = model                  (see truncation below)
$tab    = typed.tab              (tail-cut to other_usable)
$pane   = typed.pane             (tail-cut to other_usable)
if pane present:  $d3 = join(derived, " · ");  $d2 = ""
else:             $d2 = join(derived, " · ");  $d3 = ""
```

Truncation (plugin-side, so herdr never has to tail-cut a whole row and eat a later
slot):

- **Line 1.** Spare = `line1_usable − width(workspace) − (width("NN%") + 1)`. If
  `width(model) > spare`, tail-cut the model (`fab…`); if `spare ≤ 1`, clear `$model`
  for that row. The `%` is never cut and never moved. At the default width this only
  fires for workspaces ≥ 16 chars.
- **Line 2 / 3.** If `tab · derived` (or `pane · derived`) exceeds `other_usable`, keep
  the derived item whole and tail-cut the identity item to make room. A splitter exists
  only because the identity text alone does not distinguish the rows, so its tail is
  worth less than the splitter. This is the one place identity is cut for something
  other than the line edge; it is rare (long tab in a collision group) and bounded.
- A thin row's hint has line 2 to itself; no truncation interaction.

Everything else in §5 holds verbatim: whole-fleet snapshot, full reports (all 8 keys),
per-pane metadata cache, idempotent skip.

### 3.4 Fixture changes (§11)

- Delete fixture 20 `model_floats_to_line1` and 22 `line_class_order_is_typed_derived_model`.
- Replace 21 with `each_field_has_one_token`: for every fixture, `$tab` contains only
  the tab, `$pane` only the pane, `$model` only the model, `$d2`/`$d3` only derived
  items, and at most one of `$d2`/`$d3` is non-empty.
- Add `model_never_on_lines_2_3` (trivially true by construction; guards the regression),
  `tab_never_on_line_1`, `pane_never_on_line_2`, `derived_follows_pane_when_present`,
  `long_workspace_cuts_model_not_pct`, `thin_row_hint_is_d2`.
- Fixture 23 (the full-fleet `insta` snapshot) is regenerated from the §1-A mock above
  and reviewed against it line by line.

---

## 4. What this supersedes in `naming-framing.md`

| in naming-framing | status |
|---|---|
| §2 field table, `ctx%` "anchored end of line 1" | **superseded** — `%` is line 1 slot 2, immediately after the workspace; the model follows it |
| §7.1 constraints | unchanged |
| §7.2 option (c) "one token per class per line", the 12-key token set, the row template | **superseded** by §3.1–3.2 here (8 fixed-field keys, fixed template). The reason (c) was chosen — "compactness is fully preserved" — is no longer a goal that outranks consistency |
| §7.3 field → style | unchanged (bold leader, normal typed, dim derived and model, colour only on `%`) |
| §7.4 `ctx%` bullet | superseded as above |
| §7.4 `model` bullet — "fills the earliest line with spare room… the one permitted reordering" | **killed.** The model has one fixed home. There is no permitted reordering of anything |
| §7.4 "identity is never truncated to make room for `model`" | kept, and strengthened: identity and model are never on the same line, so the question cannot arise |
| §7.4 "if `model` fits nowhere, it goes on the last line" | **superseded** — it is tail-cut or cleared on line 1 (§3.3) |
| §9 indicative renders | superseded by the §1-A mock for layout; the *fields and classes* column of §9 remains correct |
| §10 item 5 (class-per-line tokens) and item 7 (model float) | superseded |
| §11 fixtures 20, 21, 22, 23 | replaced per §3.4 |
| README "Class-per-line packing" section and the config block | to be rewritten from §3 here |

§1 principle 5 ("order is fixed; presence is conditional") is the principle this document
extends from *ordering* to *position*: a field's line and slot are fixed; only its
presence varies.

---

## 5. Honest limits

What herdr prevents, and how A copes:

- **No right-aligned `%`, no columns.** The `%` x-position varies with workspace width
  (5–14 chars in this fleet). Mitigation: it is always on the first line of an entry,
  first after the bold leader, and the only colour on screen. That is the best available
  anchor; it is not a column.
- **No padding / fixed-width separators.** The user's "fixed space separators" idea is
  not implementable literally. The vertical structure (line = kind) is the substitute.
- **Line 1 has a hard ceiling.** `workspace · NN% · model` needs `width(ws) + width(model)
  ≤ 20` at the default budgets. Workspaces of 16+ chars lose the model on that row (cut
  to `fab…` or cleared). Users can raise `[layout] assumed_width` if their sidebar is in
  fact wider — herdr never reports the live width, so the plugin's arithmetic is an
  assumption (README caveat, unchanged).
- **Three lines is the maximum.** A uses all three whenever a row has a pane name. If a
  future field needed a line, something would have to share.

Residual inconsistencies A still has — stated so nobody rediscovers them as bugs:

1. **Thin rows put a dim hint in the tab slot** (row 9: `~master~` on line 2). Same
   position, different kind. It is dim where a tab is normal, so the *style* still
   signals "derived, not typed", and it is the only case where line 2 slot 1 is not a
   tab. Alternative (a dedicated 4th line) does not exist.
2. **When `%` is absent but the model is known**, the model shifts into slot 2 on line
   1. In practice both come from the same transcript read and are present or absent
   together (row 8 has neither), so this is a theoretical case; colour vs dim keeps
   them distinguishable if it ever occurs.
3. **Entry height varies 1–3 lines** (mostly 2, 3 with a pane name). The *slots* are
   fixed, the *height* is not — inherent to any layout where line 3 is optional. The
   bold-icon line 1 at the left margin is what marks each entry's start.
4. **Tail-cuts are still user text hitting the assumed width** (`remaining lci
   elephant…`, `herdr-agent-info-plugi…`). Unchanged from the framing; the fix is
   `assumed_width`, not layout.
5. **Six more lines than today** on this fleet (32 vs ~26). Accepted deliberately; if a
   fleet outgrows the sidebar, the lever is herdr's scroll, not a return to floating.
