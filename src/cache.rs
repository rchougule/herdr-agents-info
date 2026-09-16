//! Per-pane metadata cache (§5.2 rule 3 of `docs/naming-framing.md`).
//!
//! One JSON file per pane at `$HERDR_PLUGIN_STATE_DIR/<pane_id>.json`, holding
//! the pane's `model` and `ctx%` (written on every transcript read) and the last
//! emitted [`RowTokens`] (the 8-token map, `docs/layout-design.md` §3.1) for the
//! idempotent-skip in §5.2 rule 5 of `docs/naming-framing.md`. Sibling rows
//! read `model`/`ctx` from here and never re-read a transcript; a cache miss
//! means that pane has never had a transcript read, so clearing is a no-op.
//!
//! All I/O is best-effort: a hook must never take the sidebar down, so read and
//! write errors are swallowed (a miss simply yields no cached metadata).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::pack::RowTokens;

/// One pane's cached facts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneCache {
    /// Short model form (`opus`), the raw transcript reading (the `show_model`
    /// gate is applied at pack time, not here).
    #[serde(default)]
    pub model: Option<String>,
    /// Context percentage.
    #[serde(default)]
    pub pct: Option<u8>,
    /// Last measured disk footprint in bytes (raw; the `warn_mb` gate and
    /// human-readable formatting are applied at pack time, so a threshold change
    /// takes effect on the next compute without a re-measure). Every report
    /// (both `sweep` phases and `enrich`) reads this from the cache; only the
    /// sweep's disk-refresh pass measures and writes it.
    #[serde(default)]
    pub disk_bytes: Option<u64>,
    /// Unix seconds when `disk_bytes` was last measured, for the `refresh_secs`
    /// TTL. `None` means never measured (so the next sweep will measure it).
    #[serde(default)]
    pub disk_measured_at: Option<u64>,
    /// The last emitted owned-token map.
    #[serde(default)]
    pub tokens: RowTokens,
}

/// The plugin state directory (`$HERDR_PLUGIN_STATE_DIR`), when herdr set it.
pub fn dir() -> Option<PathBuf> {
    std::env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

fn path(dir: &Path, pane_id: &str) -> PathBuf {
    dir.join(format!("{pane_id}.json"))
}

/// Load a pane's cache entry, or `None` on a miss / unreadable / malformed file.
pub fn load(dir: &Path, pane_id: &str) -> Option<PaneCache> {
    let s = std::fs::read_to_string(path(dir, pane_id)).ok()?;
    serde_json::from_str(&s).ok()
}

/// Persist a pane's cache entry (best effort; errors are swallowed).
pub fn store(dir: &Path, pane_id: &str, entry: &PaneCache) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(s) = serde_json::to_string(entry) {
        let _ = std::fs::write(path(dir, pane_id), s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "agents-info-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn round_trips_an_entry() {
        let d = scratch();
        let tokens = RowTokens {
            model: "opus".into(),
            ctx_ok: "44%".into(),
            ..RowTokens::default()
        };
        let entry = PaneCache {
            model: Some("opus".into()),
            pct: Some(44),
            disk_bytes: Some(1_234_567),
            disk_measured_at: Some(1_700_000_000),
            tokens,
        };
        store(&d, "w1:p1", &entry);
        assert_eq!(load(&d, "w1:p1"), Some(entry));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn miss_is_none() {
        let d = scratch();
        assert_eq!(load(&d, "nope:p9"), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
