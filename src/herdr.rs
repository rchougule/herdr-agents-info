//! `HerdrClient` trait + a `CliClient` that shells out to `$HERDR_BIN_PATH`
//! (fallback `herdr`) and parses the JSON responses. The trait
//! exists so tests can inject a fake.

use std::io::{self, ErrorKind};
use std::process::Command;

use serde::de::DeserializeOwned;

use crate::model::{AgentInfo, PaneInfo, TabInfo, WorkspaceInfo};

/// The fixed `--source` string for every report. One source slot.
pub const SOURCE: &str = "plugin:rchougule.agents-info";

/// The set of herdr reads/reports the plugin needs. Kept minimal and injectable.
pub trait HerdrClient {
    fn agent_list(&self) -> io::Result<Vec<AgentInfo>>;
    fn pane_list(&self) -> io::Result<Vec<PaneInfo>>;
    fn tab_list(&self) -> io::Result<Vec<TabInfo>>;
    fn workspace_list(&self) -> io::Result<Vec<WorkspaceInfo>>;
    /// Patch `set` tokens and remove `clear` tokens for a pane, with a `seq`
    /// (unix millis) so stale concurrent reports are ignored.
    fn report_metadata(
        &self,
        pane_id: &str,
        set: &[(String, String)],
        clear: &[String],
        seq: u64,
    ) -> io::Result<()>;
}

/// Build the exact `pane report-metadata` argv (everything after the binary).
/// Pure so the integration test can assert it byte-for-byte.
pub fn build_report_argv(
    pane_id: &str,
    set: &[(String, String)],
    clear: &[String],
    seq: u64,
) -> Vec<String> {
    let mut argv = vec![
        "pane".to_string(),
        "report-metadata".to_string(),
        pane_id.to_string(),
        "--source".to_string(),
        SOURCE.to_string(),
    ];
    for (k, v) in set {
        argv.push("--token".to_string());
        argv.push(format!("{k}={v}"));
    }
    for k in clear {
        argv.push("--clear-token".to_string());
        argv.push(k.clone());
    }
    argv.push("--seq".to_string());
    argv.push(seq.to_string());
    argv
}

/// Real client: spawns the herdr CLI.
pub struct CliClient {
    bin: String,
}

impl Default for CliClient {
    fn default() -> Self {
        Self::new()
    }
}

impl CliClient {
    pub fn new() -> Self {
        let bin = std::env::var("HERDR_BIN_PATH")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "herdr".to_string());
        Self { bin }
    }

    /// Run a herdr subcommand, returning parsed stdout as JSON.
    fn run_json(&self, args: &[&str]) -> io::Result<serde_json::Value> {
        let out = Command::new(&self.bin).args(args).output()?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(io::Error::other(format!(
                "`{} {}` failed: {}",
                self.bin,
                args.join(" "),
                stderr.trim()
            )));
        }
        serde_json::from_slice(&out.stdout)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e.to_string()))
    }

    /// Parse `result.<field>` of a wrapped success response into a Vec.
    fn list<T: DeserializeOwned>(&self, args: &[&str], field: &str) -> io::Result<Vec<T>> {
        let v = self.run_json(args)?;
        let arr = v
            .get("result")
            .and_then(|r| r.get(field))
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        serde_json::from_value(arr)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e.to_string()))
    }
}

impl HerdrClient for CliClient {
    fn agent_list(&self) -> io::Result<Vec<AgentInfo>> {
        self.list(&["agent", "list"], "agents")
    }
    fn pane_list(&self) -> io::Result<Vec<PaneInfo>> {
        self.list(&["pane", "list"], "panes")
    }
    fn tab_list(&self) -> io::Result<Vec<TabInfo>> {
        self.list(&["tab", "list"], "tabs")
    }
    fn workspace_list(&self) -> io::Result<Vec<WorkspaceInfo>> {
        self.list(&["workspace", "list"], "workspaces")
    }
    fn report_metadata(
        &self,
        pane_id: &str,
        set: &[(String, String)],
        clear: &[String],
        seq: u64,
    ) -> io::Result<()> {
        let argv = build_report_argv(pane_id, set, clear, seq);
        let status = Command::new(&self.bin).args(&argv).status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "report-metadata for {pane_id} exited with {status}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_matches_spec_example() {
        // Worked example.
        let set = vec![
            ("name".to_string(), "auth-mw".to_string()),
            ("sub".to_string(), "dashboard".to_string()),
            ("model".to_string(), "opus".to_string()),
            ("ctx_warn".to_string(), "44%".to_string()),
        ];
        let clear = vec!["ctx_ok".to_string(), "ctx_hot".to_string()];
        let argv = build_report_argv("w1:p3", &set, &clear, 1_725_000_000_000);
        assert_eq!(
            argv,
            vec![
                "pane",
                "report-metadata",
                "w1:p3",
                "--source",
                "plugin:rchougule.agents-info",
                "--token",
                "name=auth-mw",
                "--token",
                "sub=dashboard",
                "--token",
                "model=opus",
                "--token",
                "ctx_warn=44%",
                "--clear-token",
                "ctx_ok",
                "--clear-token",
                "ctx_hot",
                "--seq",
                "1725000000000",
            ]
        );
    }
}
