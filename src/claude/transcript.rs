//! Claude transcript tail reader.
//!
//! Never full-parses a transcript (large sessions can exceed 20 MB). Reads
//! the last 256 KiB, splits on `\n`, and scans from the end for the first entry
//! with `type == "assistant"`, `message.usage` present, and `isSidechain != true`.
//! Grows the window ×4 up to a 16 MiB cap, then gives up.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const INITIAL_WINDOW: u64 = 256 * 1024;
const MAX_WINDOW: u64 = 16 * 1024 * 1024;

/// A usage reading extracted from an assistant transcript entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageEntry {
    /// input + cache_read + cache_creation.
    pub used: u64,
    /// Raw `message.model`, e.g. `claude-opus-4-8`.
    pub model_id: String,
}

impl UsageEntry {
    /// Short display form of the model (`opus` / `sonnet` / `haiku` / …).
    pub fn model_short(&self) -> String {
        model_short(&self.model_id)
    }
}

/// The tail-derived facts from one transcript scan: the last real usage entry
/// and the last manual `/rename` title.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tail {
    pub usage: Option<UsageEntry>,
    /// The latest `custom-title` value — Claude Code's manual `/rename`. `None`
    /// for a never-renamed session; never the auto-generated `aiTitle`.
    pub custom_title: Option<String>,
}

/// Read the transcript tail once, returning both the last usage entry and the
/// last manual `/rename` title, growing the window ×4 up to the 16 MiB cap (for
/// the usage). The custom title is taken from the same tail — its `custom-title`
/// lines recur, so the latest sits near the end — adding no extra reads.
pub fn read_tail(path: &Path) -> Tail {
    let Ok(mut file) = File::open(path) else {
        return Tail::default();
    };
    let Ok(meta) = file.metadata() else {
        return Tail::default();
    };
    let file_len = meta.len();
    if file_len == 0 {
        return Tail::default();
    }

    let mut window = INITIAL_WINDOW;
    let mut custom_title: Option<String> = None;
    loop {
        let effective = window.min(file_len);
        let start = file_len - effective;
        if file.seek(SeekFrom::Start(start)).is_err() {
            return Tail {
                usage: None,
                custom_title,
            };
        }
        let mut buf = vec![0u8; effective as usize];
        if file.read_exact(&mut buf).is_err() {
            return Tail {
                usage: None,
                custom_title,
            };
        }

        // When we did not start at byte 0, the first line is likely partial; drop it.
        let (usage, ct) = scan_tail_full(&buf, start > 0);
        // The latest custom title sits at the tail, present in every window, so
        // the first one we ever see is the right one — keep it across growth.
        if custom_title.is_none() {
            custom_title = ct;
        }
        if usage.is_some() {
            return Tail {
                usage,
                custom_title,
            };
        }

        // Whole file already covered, or we hit the cap → give up on usage.
        if effective >= file_len || window >= MAX_WINDOW {
            return Tail {
                usage: None,
                custom_title,
            };
        }
        window = (window.saturating_mul(4)).min(MAX_WINDOW);
    }
}

/// Read the last usage entry from a transcript file (see [`read_tail`]).
pub fn read_last_usage(path: &Path) -> Option<UsageEntry> {
    read_tail(path).usage
}

/// Scan a byte window from the end for the last qualifying assistant usage entry
/// (usage-only; see [`scan_tail_full`]). Kept for callers/tests that only need
/// the usage.
pub fn scan_tail(buf: &[u8], drop_first_partial: bool) -> Option<UsageEntry> {
    scan_tail_full(buf, drop_first_partial).0
}

/// Scan a byte window from the end for the last usage entry and the last
/// `custom-title`, parsing each line once. `drop_first_partial` drops everything
/// before the first newline (a line cut by the window boundary).
pub fn scan_tail_full(
    buf: &[u8],
    drop_first_partial: bool,
) -> (Option<UsageEntry>, Option<String>) {
    let mut slice = buf;
    if drop_first_partial {
        if let Some(nl) = slice.iter().position(|&b| b == b'\n') {
            slice = &slice[nl + 1..];
        } else {
            // No newline in the window at all: the whole thing is one partial line.
            return (None, None);
        }
    }
    let mut usage = None;
    let mut custom_title = None;
    for line in slice.split(|&b| b == b'\n').rev() {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if usage.is_none() {
            usage = usage_from_value(&v);
        }
        if custom_title.is_none() {
            custom_title = custom_title_from_value(&v);
        }
        if usage.is_some() && custom_title.is_some() {
            break;
        }
    }
    (usage, custom_title)
}

/// A `UsageEntry` from a parsed entry, only when it is a non-sidechain assistant
/// entry carrying `message.usage`.
fn usage_from_value(v: &serde_json::Value) -> Option<UsageEntry> {
    if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    // isSidechain defaults to false when absent; skip only when explicitly true.
    if v.get("isSidechain").and_then(|s| s.as_bool()) == Some(true) {
        return None;
    }
    let message = v.get("message")?;
    let usage = message.get("usage")?;
    if !usage.is_object() {
        return None;
    }
    let field = |k: &str| usage.get(k).and_then(|n| n.as_u64()).unwrap_or(0);
    let used = field("input_tokens")
        + field("cache_read_input_tokens")
        + field("cache_creation_input_tokens");
    let model_id = message.get("model").and_then(|m| m.as_str()).unwrap_or("");
    // Claude Code writes synthetic assistant messages (interrupted turns, error
    // placeholders, compact boundaries) with `model: "<synthetic>"` and a usage
    // block that does not reflect a real model or the live context. Skipping
    // them makes the scan fall back to the last *real* assistant usage, instead
    // of surfacing a `<synth…>` model at a bogus 0%.
    if model_id.starts_with('<') {
        return None;
    }
    Some(UsageEntry {
        used,
        model_id: model_id.to_string(),
    })
}

/// The manual `/rename` title from a parsed entry: a `{"type":"custom-title",
/// "customTitle":"…"}` line. The auto-generated `aiTitle` is deliberately not
/// read.
fn custom_title_from_value(v: &serde_json::Value) -> Option<String> {
    if v.get("type").and_then(|t| t.as_str()) != Some("custom-title") {
        return None;
    }
    v.get("customTitle")
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Model short form: opus/sonnet/haiku, else the segment after
/// `claude-`, else the raw id — truncated to 8 chars in the fallback.
pub fn model_short(model_id: &str) -> String {
    let lower = model_id.to_ascii_lowercase();
    for known in ["opus", "sonnet", "haiku"] {
        if lower.contains(known) {
            return known.to_string();
        }
    }
    let seg = model_id
        .strip_prefix("claude-")
        .and_then(|rest| rest.split('-').next())
        .unwrap_or(model_id);
    seg.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts")
    }

    fn used_pct(name: &str, window: u64) -> (u64, String) {
        let entry =
            read_last_usage(&fixtures().join(name)).unwrap_or_else(|| panic!("no usage in {name}"));
        (
            entry.used,
            format!("{}%", crate::claude::window::pct(entry.used, window)),
        )
    }

    #[test]
    fn model_short_forms() {
        assert_eq!(model_short("claude-opus-4-8"), "opus");
        assert_eq!(model_short("claude-sonnet-4-5-20250929"), "sonnet");
        assert_eq!(model_short("claude-haiku-4-5"), "haiku");
        assert_eq!(model_short("claude-3-5-sonnet"), "sonnet");
        // Unknown: segment after `claude-`.
        assert_eq!(model_short("claude-fabulous-2"), "fabulous");
        // Unknown, no claude- prefix: raw id truncated to 8.
        assert_eq!(model_short("gpt-5-codex"), "gpt-5-co");
    }

    #[test]
    fn skips_sidechain_and_non_assistant() {
        let bytes = concat!(
            r#"{"type":"user","message":{"content":"hi"}}"#,
            "\n",
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-opus-4-8","usage":{"input_tokens":999999}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":2,"cache_read_input_tokens":100,"cache_creation_input_tokens":50}}}"#,
            "\n"
        )
        .as_bytes()
        .to_vec();
        let e = scan_tail(&bytes, false).unwrap();
        assert_eq!(e.used, 152);
        assert_eq!(e.model_id, "claude-opus-4-8");
    }

    #[test]
    fn tail_extracts_custom_title_and_ignores_ai_title() {
        // aiTitle (auto) must be ignored; the later custom-title (manual /rename)
        // is returned, alongside the last usage — from one scan.
        let bytes = concat!(
            r#"{"type":"aiTitle","aiTitle":"Auto Summary We Ignore"}"#,
            "\n",
            r#"{"type":"custom-title","customTitle":"caching-workloom-backend","sessionId":"s"}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":2,"cache_read_input_tokens":100,"cache_creation_input_tokens":50}}}"#,
            "\n"
        )
        .as_bytes()
        .to_vec();
        let (usage, ct) = scan_tail_full(&bytes, false);
        assert_eq!(ct.as_deref(), Some("caching-workloom-backend"));
        assert_eq!(usage.unwrap().used, 152);
    }

    #[test]
    fn tail_custom_title_is_none_when_absent() {
        let bytes = concat!(
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":10}}}"#,
            "\n"
        )
        .as_bytes()
        .to_vec();
        let (_usage, ct) = scan_tail_full(&bytes, false);
        assert_eq!(ct, None);
    }

    #[test]
    fn skips_synthetic_model_entries() {
        // A synthetic tail entry (interrupted turn) must be skipped so the last
        // real assistant usage is reported, not `<synthetic>` at a bogus 0%.
        let bytes = concat!(
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":2,"cache_read_input_tokens":100,"cache_creation_input_tokens":50}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"<synthetic>","usage":{"input_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
            "\n"
        )
        .as_bytes()
        .to_vec();
        let e = scan_tail(&bytes, false).unwrap();
        assert_eq!(e.model_id, "claude-opus-4-8");
        assert_eq!(e.used, 152);
    }

    #[test]
    fn synthetic_tail_fixture_falls_back_to_real_usage() {
        let e = read_last_usage(&fixtures().join("synthetic_tail.jsonl"))
            .expect("real usage behind the synthetic tail");
        assert_eq!(e.model_short(), "opus");
        assert_eq!(e.used, 88000); // 2 + 80000 + 7998
    }

    #[test]
    fn scans_from_end_for_latest() {
        let bytes = concat!(
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":10}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":20}}}"#,
            "\n"
        )
        .as_bytes()
        .to_vec();
        // The LAST entry wins.
        assert_eq!(scan_tail(&bytes, false).unwrap().used, 20);
    }

    #[test]
    fn drops_partial_first_line() {
        // A truncated (unparseable-as-intended) first line preceding a good one.
        let bytes = concat!(
            r#"input_tokens":123}}}"#, // partial garbage from a mid-line cut
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-haiku-4-5","usage":{"input_tokens":7}}}"#,
            "\n"
        )
        .as_bytes()
        .to_vec();
        assert_eq!(scan_tail(&bytes, true).unwrap().used, 7);
    }

    #[test]
    fn no_usage_yet_returns_none() {
        let bytes = concat!(
            r#"{"type":"user","message":{"content":"hello"}}"#,
            "\n",
            r#"{"type":"summary","summary":"x"}"#,
            "\n"
        )
        .as_bytes()
        .to_vec();
        assert!(scan_tail(&bytes, false).is_none());
    }

    // Fixture-file tests. Windows below match the fixture usage totals.
    #[test]
    fn fixture_12pct() {
        assert_eq!(used_pct("ctx_12.jsonl", 200_000).1, "12%");
    }

    #[test]
    fn fixture_44pct() {
        assert_eq!(used_pct("ctx_44.jsonl", 200_000).1, "44%");
    }

    #[test]
    fn fixture_91pct() {
        assert_eq!(used_pct("ctx_91.jsonl", 200_000).1, "91%");
    }

    #[test]
    fn fixture_haiku_model() {
        let e = read_last_usage(&fixtures().join("haiku.jsonl")).unwrap();
        assert_eq!(e.model_short(), "haiku");
    }

    #[test]
    fn fixture_no_usage_yet() {
        assert!(read_last_usage(&fixtures().join("no_usage_yet.jsonl")).is_none());
    }

    #[test]
    fn fixture_1m_past_200k() {
        let e = read_last_usage(&fixtures().join("ctx_1m_past_200k.jsonl")).unwrap();
        assert!(e.used > 200_000, "expected >200k, got {}", e.used);
        let cfg = crate::config::Config::default();
        let w = crate::claude::window::resolve_window(&e.model_id, e.used, &cfg);
        assert_eq!(w, 1_000_000); // auto-promoted
    }

    #[test]
    fn fixture_large_tail_growth() {
        // A >256 KiB file whose last usage sits just past the first 256 KiB
        // window, forcing at least one ×4 growth.
        let e = read_last_usage(&fixtures().join("large_tail.jsonl")).unwrap();
        assert_eq!(e.used, 123_456);
        assert_eq!(e.model_short(), "opus");
    }
}
