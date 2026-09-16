//! Minimal, worktree-aware git branch reader. Reads `.git/HEAD`
//! directly — no `git` subprocess — and resolves the `.git` *file* form used by
//! linked worktrees. Returns `None` for a detached HEAD (the transcript
//! `gitBranch` field is the caller's fallback).

use std::path::{Path, PathBuf};

/// Branch name of the repo containing `dir`, walking up to the repo root.
pub fn branch_of(dir: &Path) -> Option<String> {
    let mut cur: Option<&Path> = Some(dir);
    while let Some(d) = cur {
        if let Some(b) = branch_at(d) {
            return Some(b);
        }
        cur = d.parent();
    }
    None
}

/// Branch from the `.git` at exactly this directory (no walking).
fn branch_at(dir: &Path) -> Option<String> {
    let dot_git = dir.join(".git");
    let head_path = if dot_git.is_dir() {
        dot_git.join("HEAD")
    } else if dot_git.is_file() {
        // Linked worktree: `.git` is a file `gitdir: <path>`.
        let content = std::fs::read_to_string(&dot_git).ok()?;
        let gitdir = content.strip_prefix("gitdir:")?.trim();
        PathBuf::from(gitdir).join("HEAD")
    } else {
        return None;
    };
    let head = std::fs::read_to_string(head_path).ok()?;
    parse_head(&head)
}

/// Parse a `HEAD` file body into a branch name (or `None` when detached).
fn parse_head(head: &str) -> Option<String> {
    let line = head.trim();
    let rest = line.strip_prefix("ref:")?.trim();
    let branch = rest.strip_prefix("refs/heads/").unwrap_or(rest);
    (!branch.is_empty()).then(|| branch.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_symbolic_ref() {
        assert_eq!(
            parse_head("ref: refs/heads/feature/auth\n").as_deref(),
            Some("feature/auth")
        );
        assert_eq!(parse_head("ref: refs/heads/main").as_deref(), Some("main"));
    }

    #[test]
    fn detached_head_is_none() {
        assert!(parse_head("a1b2c3d4e5f6\n").is_none());
    }

    #[test]
    fn reads_plain_git_dir() {
        let base = std::env::temp_dir().join(format!("agents-info-git-{}", std::process::id()));
        let git = base.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), b"ref: refs/heads/billing\n").unwrap();
        assert_eq!(branch_of(&base).as_deref(), Some("billing"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn reads_linked_worktree_git_file() {
        let base = std::env::temp_dir().join(format!("agents-info-git-wt-{}", std::process::id()));
        let real_gitdir = base.join("realgit/worktrees/wt1");
        std::fs::create_dir_all(&real_gitdir).unwrap();
        std::fs::write(real_gitdir.join("HEAD"), b"ref: refs/heads/dash-v2\n").unwrap();
        let wt = base.join("checkout");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", real_gitdir.display()),
        )
        .unwrap();
        assert_eq!(branch_of(&wt).as_deref(), Some("dash-v2"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
