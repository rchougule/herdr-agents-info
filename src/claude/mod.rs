//! Claude Code adapter: transcript tail reader, context-window table, account
//! reader, and transcript-path resolution (PLAN §5.2 / §5.3).

pub mod account;
pub mod transcript;
pub mod window;

use std::path::PathBuf;

/// Slug a cwd the way Claude Code names its `~/.claude/projects/<slug>/` dir:
/// every `/` becomes `-` (PLAN §5.2; verified on the reference setup).
pub fn cwd_slug(cwd: &str) -> String {
    cwd.replace('/', "-")
}

/// The transcript directory for a cwd under `~/.claude/projects/`.
fn project_dir(home: &std::path::Path, cwd: &str) -> PathBuf {
    home.join(".claude/projects").join(cwd_slug(cwd))
}

/// Resolve the transcript path for a pane (PLAN §5.2), in order:
///   0. `AGENTS_INFO_FIXTURE_DIR` override → `<fixture>/<pane_id>.jsonl`.
///   1. When we have the session UUID (herdr reports it via `agent_session`),
///      the transcript is `<some project dir>/<uuid>.jsonl`. Try the cwd/
///      foreground_cwd slug dirs first (fast path), then search *all* project
///      dirs for `<uuid>.jsonl`. The UUID is globally unique, so this is always
///      the correct session. If the UUID cannot be found anywhere, return
///      `None` — we never guess a different session, because a wrong context %
///      is worse than a blank one.
///   2. Only when there is no UUID at all: newest `*.jsonl` by mtime under the
///      slug dirs (best effort for sessions started without herdr's hook).
///   3. Otherwise `None`.
pub fn resolve_transcript(
    pane_id: &str,
    cwd: Option<&str>,
    foreground_cwd: Option<&str>,
    session_uuid: Option<&str>,
) -> Option<PathBuf> {
    // QA / test override.
    if let Some(dir) = std::env::var_os("AGENTS_INFO_FIXTURE_DIR") {
        let p = PathBuf::from(dir).join(format!("{pane_id}.jsonl"));
        return p.exists().then_some(p);
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    // Prefer foreground_cwd first (the session may have `cd`'d), then cwd.
    let dirs: Vec<&str> = [foreground_cwd, cwd].into_iter().flatten().collect();

    // 1. Resolve strictly by session UUID.
    if let Some(uuid) = session_uuid.filter(|u| !u.is_empty()) {
        // Fast path: the expected slug dir(s).
        for cwd in &dirs {
            let p = project_dir(&home, cwd).join(format!("{uuid}.jsonl"));
            if p.exists() {
                return Some(p);
            }
        }
        // Slug missed (worktree / renamed / `cd`'d dir): find the file by UUID
        // across every project dir. Never fall through to a "newest file" guess.
        return find_transcript_by_uuid(&home, uuid);
    }

    // 2. No UUID: newest *.jsonl by mtime (best effort only).
    for cwd in &dirs {
        if let Some(p) = newest_jsonl(&project_dir(&home, cwd)) {
            return Some(p);
        }
    }

    None
}

/// Search every `~/.claude/projects/<dir>/` for `<uuid>.jsonl`. The UUID is
/// unique across sessions, so a hit is unambiguously the pane's transcript even
/// when the cwd→slug mapping fails. In the unlikely event the same UUID exists
/// in more than one project dir, prefer the most recently modified.
fn find_transcript_by_uuid(home: &std::path::Path, uuid: &str) -> Option<PathBuf> {
    find_uuid_in_projects(&home.join(".claude/projects"), uuid)
}

fn find_uuid_in_projects(root: &std::path::Path, uuid: &str) -> Option<PathBuf> {
    let file = format!("{uuid}.jsonl");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let path = entry.path().join(&file);
        let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            continue; // does not exist / unreadable
        };
        if best.as_ref().is_none_or(|(t, _)| mtime > *t) {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, p)| p)
}

/// Whether herdr's own Claude Code integration hook looks installed in
/// `~/.claude/settings.json` (PLAN §9.4, open question #4: without it,
/// `agent_session` is never populated and pane→transcript mapping falls back
/// to newest-jsonl, which can pick the wrong session when two Claude
/// sessions share a cwd). We deliberately do not parse the hooks schema —
/// its shape has changed across Claude Code versions — and instead check
/// whether any hook command mentions the script herdr ships
/// (`herdr-agent-state.sh`). `None` means "unknown" (no `$HOME`, or the file
/// is missing/unreadable), which callers must not report as "not installed".
pub fn claude_hook_installed() -> Option<bool> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let text = std::fs::read_to_string(home.join(".claude/settings.json")).ok()?;
    Some(text.contains("herdr-agent-state"))
}

/// Newest `*.jsonl` in a directory by mtime, or `None` if the dir is absent/empty.
fn newest_jsonl(dir: &std::path::Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(t, _)| mtime > *t) {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_transcript_by_uuid_when_slug_dir_does_not_match() {
        // A session whose cwd-slug does NOT match the dir the file actually
        // lives in (worktree / renamed dir). The UUID search must still find it.
        let root = std::env::temp_dir().join(format!("agents-info-uuid-{}", std::process::id()));
        let unrelated = root.join("-Users-someone-unrelated-worktree");
        std::fs::create_dir_all(&unrelated).unwrap();
        let uuid = "db66f921-e1c0-41fd-8c68-22ec02c0576d";
        let want = unrelated.join(format!("{uuid}.jsonl"));
        std::fs::write(&want, b"{}").unwrap();
        // A decoy newest file in another dir must NOT win.
        let other = root.join("-Users-someone-else");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("00000000-0000-0000-0000-000000000000.jsonl"),
            b"{}",
        )
        .unwrap();

        let got = find_uuid_in_projects(&root, uuid);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(got, Some(want));
    }

    #[test]
    fn uuid_search_returns_none_when_absent() {
        let root =
            std::env::temp_dir().join(format!("agents-info-uuid-none-{}", std::process::id()));
        std::fs::create_dir_all(root.join("-x")).unwrap();
        let got = find_uuid_in_projects(&root, "no-such-uuid");
        let _ = std::fs::remove_dir_all(&root);
        assert!(got.is_none());
    }

    #[test]
    fn slug_replaces_slashes() {
        assert_eq!(
            cwd_slug("/home/user/proj/herdr-agents-info"),
            "-home-user-proj-herdr-agents-info"
        );
    }

    #[test]
    fn fixture_dir_override_maps_pane_id() {
        let dir = std::env::temp_dir().join(format!("agents-info-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("w1:p3.jsonl");
        std::fs::write(&f, b"{}").unwrap();
        // SAFETY: single-threaded test; restored below.
        std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &dir);
        let got = resolve_transcript("w1:p3", Some("/x"), None, Some("uuid"));
        std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");
        assert_eq!(got, Some(f));
    }

    #[test]
    fn fixture_dir_override_missing_file_is_none() {
        let dir = std::env::temp_dir().join(format!("agents-info-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("AGENTS_INFO_FIXTURE_DIR", &dir);
        let got = resolve_transcript("no:such", Some("/x"), None, None);
        std::env::remove_var("AGENTS_INFO_FIXTURE_DIR");
        assert!(got.is_none());
    }

    #[test]
    fn hook_installed_detects_the_script_name() {
        let home =
            std::env::temp_dir().join(format!("agents-info-home-hook-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command",
                "command":"bash '/Users/x/.claude/hooks/herdr-agent-state.sh' session"}]}]}}"#,
        )
        .unwrap();
        // SAFETY: single-threaded test; restored below. No other test in this
        // binary reads $HOME.
        let prior = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);
        let got = claude_hook_installed();
        match prior {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(got, Some(true));
    }

    #[test]
    fn hook_not_installed_when_settings_lack_the_script() {
        let home =
            std::env::temp_dir().join(format!("agents-info-home-nohook-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(home.join(".claude/settings.json"), r#"{"hooks":{}}"#).unwrap();
        let prior = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);
        let got = claude_hook_installed();
        match prior {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(got, Some(false));
    }

    #[test]
    fn hook_installed_unknown_without_settings_file() {
        let home =
            std::env::temp_dir().join(format!("agents-info-home-missing-{}", std::process::id()));
        let prior = std::env::var_os("HOME");
        std::env::set_var("HOME", &home); // dir intentionally not created
        let got = claude_hook_installed();
        match prior {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        assert_eq!(got, None);
    }
}
