# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and the project aims to follow
[Semantic Versioning](https://semver.org/).

## [0.1.0] — 2026-09-16

First release. Annotates every Claude Code row in the herdr Agents sidebar.

### Added

- **Distinguishing name** per pane via the identity ladder (workspace → tab →
  agent/pane), with a whole-fleet collision splitter (branch → directory → pane
  id) and a thin-row hint, so no two rows read alike.
- **Type prefixes** `T:` tab · `P:` pane · `A:` agent; an agent `/rename`
  (Claude Code `customTitle`) shows as `A:<name>` and wins over a pane rename.
- **Model** short form and a color-coded **context %** (green → amber → red),
  with a 1M-context auto-promotion heuristic and per-model overrides.
- **Disk footprint** (`$disk`) — a human-readable size shown only above a
  threshold, measuring the worktree (default), the transcript, or the project
  dir; the worktree walk is TTL-cached, parallel, timeout-bounded, and runs off
  the sweep's critical path so fast tokens never wait on it.
- **Configurable icons** per metadata field and a between-entry **separator**
  line; light/dark theming via the config recipe.
- `doctor` action for transcript-mapping and hook diagnostics; `sweep` action to
  refresh all rows.

[0.1.0]: https://github.com/rchougule/herdr-agents-info/releases/tag/v0.1.0
