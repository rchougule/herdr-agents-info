//! herdr `agents-info` plugin — core library.
//!
//! Pure logic + the herdr client boundary. The binary (`src/main.rs`) is a thin
//! clap wrapper over [`app`]. See `PLAN.md` for the full design.

pub mod app;
pub mod cache;
pub mod claude;
pub mod config;
pub mod disk;
pub mod git;
pub mod herdr;
pub mod model;
pub mod name;
pub mod pack;
pub mod render;

/// Unix milliseconds — the `--seq` value (PLAN §3.2).
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
