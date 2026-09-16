#!/usr/bin/env bash
# qa/scenario.sh — recreate the real pain against a *live* herdr
# server, using the herdr CLI only. Deterministic and token-free: every pane
# is registered as a "claude" agent via `pane report-agent` (a custom-hook
# report, not a real `claude` process), and its context/model state comes
# from a committed fixture transcript picked up through
# `AGENTS_INFO_FIXTURE_DIR` — no network call, no real Claude session, no
# tokens burned.
#
# Fleet (a worked example, plus state variants):
#   w1  dashboard / auth-mw    ctx  12%  opus   idle
#   w2  dashboard / billing    ctx  44%  opus   idle
#   w3  dashboard / main       (no transcript yet — fresh start)   idle
#   w4  dashboard / main (2nd) ctx  91%  opus   blocked
#   w5  api       / main       haiku, ~5% ctx                      idle
#
# This script only WRITES to a running herdr server (workspace create, pane
# report-agent) and to its own scratch state under qa/.state/ (gitignored).
# It never touches herdr's or this plugin's source.
#
# Usage:
#   qa/scenario.sh                 create the fleet
#   qa/scenario.sh --force         recreate even if a previous run's state exists
#   qa/scenario.sh --teardown      close every workspace this script created
#
# Requires: herdr (a running server — see the preflight below), git, jq.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." >/dev/null 2>&1 && pwd -P)"

STATE_DIR="$REPO_ROOT/qa/.state"
STATE_FILE="$STATE_DIR/created-workspaces.txt"
REPOS_DIR="$STATE_DIR/repos"
# The live fixture directory the *running herdr server's environment* must
# also have exported as AGENTS_INFO_FIXTURE_DIR before it was started (see
# qa/README.md) — plugin hook subprocesses inherit herdr's environment, not
# this script's. Overridable so a human can point both at the same path.
FIXTURE_LIVE_DIR="${AGENTS_INFO_FIXTURE_DIR:-$STATE_DIR/fixture-live}"
FIXTURES_SRC_DIR="$REPO_ROOT/tests/fixtures/transcripts"

SOURCE_ID="custom:qa"

usage() {
  cat <<'EOF'
Usage: qa/scenario.sh [--force] [--teardown] [-h|--help]

  (no flags)   Create the §6.1 QA fleet: 4 workspaces labelled "dashboard"
               at distinct branches/dirs, plus 1 "api" workspace. One pane
               is forced `blocked`, one sits at ~91% context, one runs
               haiku, one has no transcript yet.
  --force      Ignore existing state from a previous run (does not close
               those old workspaces first — run --teardown for that).
  --teardown   Close every workspace this script created and remove
               qa/.state/.

Requires a running herdr server (checked via `herdr status`), plus `git`
and `jq` on PATH.

IMPORTANT: export AGENTS_INFO_FIXTURE_DIR to a stable path *before starting
herdr* so the plugin's sweep/enrich hooks (which run as herdr subprocesses)
can see the fixture transcripts this script stages. See qa/README.md.
EOF
}

log() { printf '==> %s\n' "$*" >&2; }
die() {
  printf 'qa/scenario.sh: %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "'$1' is required on PATH but was not found."
}

preflight() {
  require_cmd herdr
  require_cmd git
  require_cmd jq
  # Best-effort liveness check: `herdr status` should succeed against a
  # running server. The exact output format is not part of the documented
  # CLI contract, so we only gate on exit status plus an obvious "not
  # running" phrase, and let the real herdr error surface otherwise.
  local status_out status_rc
  status_out="$(herdr status 2>&1)" && status_rc=0 || status_rc=$?
  if [ "$status_rc" -ne 0 ] || printf '%s' "$status_out" | grep -qiE 'not running|no server|no session'; then
    die "herdr does not look reachable. Start/attach a herdr session first, then re-run.
herdr status said:
$status_out"
  fi
}

# create_git_repo DIR BRANCH — a minimal real git repo checked out on BRANCH,
# used only so src/git.rs's HEAD reader has something real to resolve. No
# remote, no herdr involvement.
create_git_repo() {
  local dir="$1" branch="$2"
  mkdir -p "$dir"
  if [ ! -d "$dir/.git" ]; then
    git -C "$dir" -c init.defaultBranch="$branch" init -q
  fi
  git -C "$dir" -c user.email=qa@herdr.local -c user.name=herdr-qa \
    checkout -q -B "$branch"
  git -C "$dir" -c user.email=qa@herdr.local -c user.name=herdr-qa \
    commit -q --allow-empty -m "qa fixture ($branch)"
}

# herdr_json ARGS... — run a herdr subcommand and print its JSON stdout.
herdr_json() {
  herdr "$@"
}

record_workspace() {
  mkdir -p "$STATE_DIR"
  printf '%s\n' "$1" >>"$STATE_FILE"
}

stage_fixture() {
  local pane_id="$1" fixture_file="$2"
  if [ "$fixture_file" = "-" ]; then
    return 0 # deliberately no transcript (the "fresh start" pane)
  fi
  mkdir -p "$FIXTURE_LIVE_DIR"
  cp "$FIXTURES_SRC_DIR/$fixture_file" "$FIXTURE_LIVE_DIR/$pane_id.jsonl"
}

create_pane() {
  local repo_dirname="$1" branch="$2" label="$3" agent_state="$4" fixture_file="$5"
  local dir="$REPOS_DIR/$repo_dirname"
  create_git_repo "$dir" "$branch"

  log "workspace create --cwd $dir --label $label"
  local resp workspace_id pane_id
  resp="$(herdr_json workspace create --cwd "$dir" --label "$label" --no-focus)"
  workspace_id="$(printf '%s' "$resp" | jq -r '.result.workspace.workspace_id')"
  pane_id="$(printf '%s' "$resp" | jq -r '.result.root_pane.pane_id')"
  [ -n "$workspace_id" ] && [ "$workspace_id" != "null" ] || die "workspace create did not return a workspace_id: $resp"
  [ -n "$pane_id" ] && [ "$pane_id" != "null" ] || die "workspace create did not return a root pane_id: $resp"
  record_workspace "$workspace_id"

  stage_fixture "$pane_id" "$fixture_file"

  log "pane report-agent $pane_id --agent claude --state $agent_state"
  herdr pane report-agent "$pane_id" \
    --source "$SOURCE_ID" \
    --agent claude \
    --state "$agent_state"

  printf '  %-8s %-9s branch=%-8s state=%-8s pane=%s\n' "$label" "$repo_dirname" "$branch" "$agent_state" "$pane_id"
}

do_create() {
  if [ -f "$STATE_FILE" ] && [ "$FORCE" != "1" ]; then
    die "state already exists at $STATE_FILE (a previous run's fleet may still be live). Re-run with --force to add more, or --teardown first."
  fi
  mkdir -p "$STATE_DIR" "$REPOS_DIR"

  log "fixture-live dir: $FIXTURE_LIVE_DIR"
  if [ -z "${AGENTS_INFO_FIXTURE_DIR:-}" ]; then
    log "WARNING: AGENTS_INFO_FIXTURE_DIR is not set in *this* shell. That is" \
      "fine for staging fixtures here, but the herdr server process must" \
      "have this same path exported before it was started, or its plugin" \
      "hooks will not see these fixtures. See qa/README.md."
  fi

  log "creating the §6.1 fleet..."
  # dir-name  branch        label       agent-state  fixture
  create_pane "auth-mw/dashboard" "auth-mw" "dashboard" "idle" "ctx_12.jsonl"
  create_pane "billing/dashboard" "billing" "dashboard" "idle" "ctx_44.jsonl"
  create_pane "main-1/dashboard" "main" "dashboard" "idle" "-"
  create_pane "main-2/dash-v2" "main" "dashboard" "blocked" "ctx_91.jsonl"
  create_pane "api/api" "main" "api" "idle" "haiku.jsonl"

  log "done. Trigger a sweep to populate the sidebar now (see qa/README.md):"
  log "  herdr plugin action invoke sweep --plugin rchougule.agents-info"
  log "or restart/hand off the herdr server to exercise the [[startup]] hook."
}

do_teardown() {
  if [ ! -f "$STATE_FILE" ]; then
    log "no state file at $STATE_FILE; nothing to tear down."
  else
    while IFS= read -r workspace_id; do
      [ -n "$workspace_id" ] || continue
      log "workspace close $workspace_id"
      herdr workspace close "$workspace_id" || log "  (already gone, ignoring)"
    done <"$STATE_FILE"
  fi
  log "removing $STATE_DIR"
  rm -rf "$STATE_DIR"
}

FORCE=0
MODE=create
while [ $# -gt 0 ]; do
  case "$1" in
  --force)
    FORCE=1
    shift
    ;;
  --teardown)
    MODE=teardown
    shift
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  *)
    die "unknown argument: $1 (see --help)"
    ;;
  esac
done

preflight

case "$MODE" in
create) do_create ;;
teardown) do_teardown ;;
esac
