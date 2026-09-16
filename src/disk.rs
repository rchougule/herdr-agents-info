//! Per-pane disk footprint (the `$disk` token).
//!
//! Agents that spawn subagents and throwaway git worktrees accumulate a lot of
//! disk — each worktree carries its own `node_modules` / `target` etc., which is
//! where the GB live. This surfaces a glanceable, human-readable footprint per
//! Claude pane so a heavy session is obvious before cleanup. The token is only
//! ever *set* above the configured threshold — below it, it clears (an alert,
//! not clutter).
//!
//! Three measures:
//!   - [`Measure::Cwd`] (default) — the recursive size of the pane's working
//!     directory (`foreground_cwd`, falling back to `cwd`), i.e. the worktree
//!     checkout. This is the one that catches the GB pain, but a full tree walk
//!     is expensive, so it runs behind a TTL cache and a per-tree timeout, off
//!     the sweep's critical path (see `app.rs` two-phase sweep).
//!   - [`Measure::Transcript`] — the single session `.jsonl` (one `stat`).
//!     Subagent turns fold into this file as `isSidechain` entries, so they
//!     already count; but transcripts are typically only MB.
//!   - [`Measure::ProjectDir`] — the sum of every `.jsonl` in the transcript's
//!     `~/.claude/projects/<slug>/` dir. Captures sibling sessions in the same
//!     cwd, at the cost of over-attributing shared history to each such pane.
//!
//! All I/O is best-effort — any error yields `None`, never a panic (a hook must
//! never take the sidebar down). The walk never follows symlinks (no cycles, no
//! double-counting) and never shells out.

use std::path::Path;
use std::time::Instant;

/// What a pane's disk footprint is measured over. Parsed from `[disk] measure`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Measure {
    /// The pane's working directory (worktree checkout), measured recursively.
    #[default]
    Cwd,
    /// The session's own transcript file only.
    Transcript,
    /// Every `.jsonl` in the transcript's `~/.claude/projects/<slug>/` dir.
    ProjectDir,
}

impl Measure {
    /// Parse the `[disk] measure` value; anything unrecognised → `Cwd` (default).
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "transcript" => Measure::Transcript,
            "project_dir" | "project" | "projectdir" => Measure::ProjectDir,
            // "cwd" and anything else → the default.
            _ => Measure::Cwd,
        }
    }

    /// Whether this measure is a potentially-expensive recursive tree walk
    /// (and so needs the TTL cache + timeout + off-critical-path handling).
    pub fn is_tree_walk(self) -> bool {
        matches!(self, Measure::Cwd)
    }
}

/// The outcome of a measurement: the size in bytes (when obtained) and whether
/// the walk hit its deadline before finishing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sized {
    pub bytes: Option<u64>,
    pub timed_out: bool,
}

impl Sized {
    fn bytes(b: u64) -> Self {
        Sized {
            bytes: Some(b),
            timed_out: false,
        }
    }
    fn none() -> Self {
        Sized {
            bytes: None,
            timed_out: false,
        }
    }
}

/// Measure a pane's disk footprint from `target`, best-effort, giving up on the
/// recursive walk once `deadline` passes.
///
/// `target` is the resolved thing to measure for the mode: the cwd directory for
/// [`Measure::Cwd`], otherwise the resolved transcript file (whose parent dir is
/// summed for [`Measure::ProjectDir`]). A missing/unreadable target yields
/// `bytes: None`.
pub fn measure(target: &Path, measure: Measure, deadline: Instant) -> Sized {
    match measure {
        Measure::Transcript => std::fs::metadata(target)
            .ok()
            .map(|m| Sized::bytes(m.len()))
            .unwrap_or_else(Sized::none),
        Measure::ProjectDir => {
            let Some(dir) = target.parent() else {
                return Sized::none();
            };
            let Ok(rd) = std::fs::read_dir(dir) else {
                return Sized::none();
            };
            let mut total: u64 = 0;
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                if let Ok(meta) = entry.metadata() {
                    total = total.saturating_add(meta.len());
                }
            }
            Sized::bytes(total)
        }
        Measure::Cwd => dir_size_bounded(target, deadline),
    }
}

/// Recursively sum the sizes of regular files under `root`, giving up with
/// `timed_out: true` (and `bytes: None`) once `deadline` passes. Iterative (an
/// explicit stack, so a deep tree cannot blow the call stack); never follows
/// symlinks (avoids cycles and double-counting); best-effort per entry.
pub fn dir_size_bounded(root: &Path, deadline: Instant) -> Sized {
    if !root.is_dir() {
        return Sized::none();
    }
    let mut total: u64 = 0;
    let mut stack = vec![root.to_path_buf()];
    let mut checked: u32 = 0;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue; // unreadable dir → skip, keep going
        };
        for entry in rd.flatten() {
            // Amortised deadline check: cheap, and frequent enough that a
            // pathological tree bails within a bounded overshoot.
            checked = checked.wrapping_add(1);
            if checked.is_multiple_of(512) && Instant::now() >= deadline {
                return Sized {
                    bytes: None,
                    timed_out: true,
                };
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                if let Ok(meta) = entry.metadata() {
                    total = total.saturating_add(meta.len());
                }
            }
        }
    }
    Sized::bytes(total)
}

/// Human-readable size for a glance token: `B` / `K` / `M` / `G` (1024-based),
/// with one decimal below 10 of a unit (`5.0M`, `1.2G`) and a whole number at or
/// above it (`340M`, `900K`). Bytes render exact (`512B`); zero is `0B`.
pub fn human_readable(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("G", 1 << 30), ("M", 1 << 20), ("K", 1 << 10)];
    for (suffix, scale) in UNITS {
        if bytes >= scale {
            let v = bytes as f64 / scale as f64;
            return if v < 10.0 {
                format!("{v:.1}{suffix}")
            } else {
                format!("{}{suffix}", v.round() as u64)
            };
        }
    }
    format!("{bytes}B")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(3600)
    }

    #[test]
    fn human_readable_forms() {
        assert_eq!(human_readable(0), "0B");
        assert_eq!(human_readable(512), "512B");
        assert_eq!(human_readable(1023), "1023B");
        assert_eq!(human_readable(1 << 10), "1.0K");
        assert_eq!(human_readable(900 * (1 << 10)), "900K");
        assert_eq!(human_readable(5 * (1 << 20)), "5.0M");
        assert_eq!(human_readable(340 * (1 << 20)), "340M");
        assert_eq!(human_readable(3 * (1 << 30) / 2), "1.5G"); // 1.5 GiB
        assert_eq!(human_readable(12 * (1 << 30)), "12G");
    }

    #[test]
    fn measure_parse_defaults_to_cwd() {
        assert_eq!(Measure::parse("cwd"), Measure::Cwd);
        assert_eq!(Measure::parse("transcript"), Measure::Transcript);
        assert_eq!(Measure::parse("project_dir"), Measure::ProjectDir);
        assert_eq!(Measure::parse("PROJECT"), Measure::ProjectDir);
        assert_eq!(Measure::parse("whatever"), Measure::Cwd);
        assert!(Measure::Cwd.is_tree_walk());
        assert!(!Measure::Transcript.is_tree_walk());
    }

    #[test]
    fn measures_transcript_and_project_dir() {
        let dir = std::env::temp_dir().join(format!("agents-info-disk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("session-a.jsonl");
        let b = dir.join("session-b.jsonl");
        std::fs::write(&a, vec![b'x'; 1000]).unwrap();
        std::fs::write(&b, vec![b'y'; 2000]).unwrap();
        // A non-transcript file must not count toward the project-dir total.
        std::fs::write(dir.join("notes.txt"), vec![b'z'; 5000]).unwrap();

        assert_eq!(measure(&a, Measure::Transcript, far()).bytes, Some(1000));
        assert_eq!(measure(&a, Measure::ProjectDir, far()).bytes, Some(3000));

        let missing = dir.join("nope.jsonl");
        assert_eq!(measure(&missing, Measure::Transcript, far()).bytes, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cwd_walk_sums_recursively() {
        let root = std::env::temp_dir().join(format!("agents-info-walk-{}", std::process::id()));
        let sub = root.join("nested/deep");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("a.txt"), vec![b'x'; 100]).unwrap();
        std::fs::write(root.join("nested/b.txt"), vec![b'y'; 200]).unwrap();
        std::fs::write(sub.join("c.txt"), vec![b'z'; 300]).unwrap();

        let s = measure(&root, Measure::Cwd, far());
        assert_eq!(s.bytes, Some(600));
        assert!(!s.timed_out);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cwd_walk_missing_dir_is_none() {
        let missing =
            std::env::temp_dir().join(format!("agents-info-nodir-{}", std::process::id()));
        let s = measure(&missing, Measure::Cwd, far());
        assert_eq!(s.bytes, None);
        assert!(!s.timed_out);
    }

    #[test]
    fn cwd_walk_times_out_on_a_deadline_in_the_past() {
        let root = std::env::temp_dir().join(format!("agents-info-to-{}", std::process::id()));
        // Enough entries that the %512 deadline check is reached.
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..1200 {
            std::fs::write(root.join(format!("f{i}.txt")), b"x").unwrap();
        }
        let past = Instant::now() - Duration::from_secs(1);
        let s = dir_size_bounded(&root, past);
        assert!(s.timed_out);
        assert_eq!(s.bytes, None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
