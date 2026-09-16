#!/usr/bin/env bash
# qa/shoot.sh — macOS screencapture driver for the §6.3 QA shots.
#
# Captures either an explicit screen region (the sidebar only) or a whole
# terminal-app window, and saves it to qa/out/<variant>.png. Coordinates and
# window identity are machine-specific (they depend on your screen layout,
# terminal app, and font size), so both are parameterized via flags/env —
# see "Finding your coordinates" below and in qa/README.md.
#
# This script only captures pixels already on screen. It does not toggle
# herdr's sidebar width, sort mode, or theme — those are config.toml settings
# that need a herdr restart/live-handoff to take effect (no CLI toggle
# exists for them), so change config.toml + restart between invocations for
# each §6.3 variant (expanded/collapsed, grouped/flat, dark/light, widths
# 18/26/36) and shoot once per resulting state.
#
# Usage:
#   qa/shoot.sh --variant NAME --region X,Y,W,H
#   qa/shoot.sh --variant NAME --app "Terminal"
#   qa/shoot.sh --variant NAME --app "iTerm2"
#   qa/shoot.sh --list
#
# Env alternatives to the flags above:
#   AGENTS_INFO_SHOOT_REGION="X,Y,W,H"
#   AGENTS_INFO_SHOOT_APP="Terminal"
#
# Finding your coordinates:
#   Region:  run `screencapture -i qa/out/_probe.png` once, drag a box over
#            the sidebar; that just saves a PNG, but macOS shows the
#            live x,y w×h readout in the menu bar while dragging — copy it
#            into --region. (`screencapture -i` is interactive-only; there
#            is no CLI flag that prints the box without also capturing it.)
#   Window:  --app resolves a CGWindowID via AppleScript (`id of window 1`
#            of the named app), which `screencapture -l` accepts directly.
#            Works for Terminal.app and iTerm2. Requires Screen Recording
#            permission for the app running this script (System Settings ->
#            Privacy & Security -> Screen Recording).
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." >/dev/null 2>&1 && pwd -P)"
OUT_DIR="$REPO_ROOT/qa/out"

# The full §6.3 variant matrix, for --list / reference. shoot.sh does not
# loop over these itself (each needs a manual config.toml + restart step
# between shots); it names the shot you are about to take.
VARIANTS=(
  "expanded-grouped-dark-w18" "expanded-grouped-dark-w26" "expanded-grouped-dark-w36"
  "expanded-grouped-light-w18" "expanded-grouped-light-w26" "expanded-grouped-light-w36"
  "expanded-flat-dark-w18" "expanded-flat-dark-w26" "expanded-flat-dark-w36"
  "expanded-flat-light-w18" "expanded-flat-light-w26" "expanded-flat-light-w36"
  "collapsed-dark" "collapsed-light"
)

usage() {
  cat <<EOF
Usage: qa/shoot.sh --variant NAME (--region X,Y,W,H | --app APP_NAME) [--out DIR]
       qa/shoot.sh --list

  --variant NAME   Shot label; saved to qa/out/NAME.png. Suggested names
                   are listed by --list.
  --region X,Y,W,H Explicit screen region in points (screencapture -R).
  --app APP_NAME   Capture APP_NAME's frontmost window by CGWindowID
                   (screencapture -l). Tested with "Terminal" and "iTerm2".
  --out DIR        Output directory (default: qa/out).
  --list           Print the suggested variant names and exit.

Exactly one of --region / --app is required (or the matching env var —
see the script header).
EOF
}

die() {
  printf 'qa/shoot.sh: %s\n' "$*" >&2
  exit 1
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  die "this script drives macOS's screencapture(1); it does not run on $(uname -s)."
fi

VARIANT=""
REGION="${AGENTS_INFO_SHOOT_REGION:-}"
APP="${AGENTS_INFO_SHOOT_APP:-}"

while [ $# -gt 0 ]; do
  case "$1" in
  --variant)
    VARIANT="${2:?--variant requires a value}"
    shift 2
    ;;
  --region)
    REGION="${2:?--region requires X,Y,W,H}"
    shift 2
    ;;
  --app)
    APP="${2:?--app requires an app name}"
    shift 2
    ;;
  --out)
    OUT_DIR="${2:?--out requires a directory}"
    shift 2
    ;;
  --list)
    printf '%s\n' "${VARIANTS[@]}"
    exit 0
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

[ -n "$VARIANT" ] || die "--variant is required (see --help)"
if [ -n "$REGION" ] && [ -n "$APP" ]; then
  die "pass only one of --region / --app (or their env equivalents), not both."
fi
[ -n "$REGION" ] || [ -n "$APP" ] || die "one of --region or --app is required (see --help)"

mkdir -p "$OUT_DIR"
DEST="$OUT_DIR/$VARIANT.png"

if [ -n "$REGION" ]; then
  IFS=',' read -r rx ry rw rh <<<"$REGION"
  [ -n "${rx:-}" ] && [ -n "${ry:-}" ] && [ -n "${rw:-}" ] && [ -n "${rh:-}" ] ||
    die "--region must be X,Y,W,H, got: $REGION"
  echo "Capturing region ${rx},${ry} ${rw}x${rh} -> $DEST" >&2
  screencapture -x -R "${rx},${ry},${rw},${rh}" "$DEST"
else
  window_id="$(osascript -e "tell application \"$APP\" to id of window 1" 2>/dev/null || true)"
  [ -n "$window_id" ] || die "could not resolve a window id for '$APP'. Is it running with a window open? (osascript needs Accessibility/Automation permission the first time.)"
  echo "Capturing $APP window (id $window_id) -> $DEST" >&2
  screencapture -x -o -l "$window_id" "$DEST"
fi

[ -s "$DEST" ] || die "screencapture did not produce $DEST (check Screen Recording permission in System Settings)."
echo "Saved $DEST" >&2
