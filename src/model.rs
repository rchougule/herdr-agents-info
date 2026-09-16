//! Serde subsets of the herdr API schema. Verified field-for-field against
//! `herdr/src/api/schema/*.rs` on 2026-09-07. All structs are tolerant of
//! unknown fields (serde ignores them by default) so newer herdr releases that
//! add fields do not break parsing.

use serde::Deserialize;

/// `AgentInfo` subset (`agents.rs:184`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentInfo {
    pub pane_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub agent: Option<String>,
    /// herdr agent name (`agent start <name>`).
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub agent_session: Option<AgentSessionInfo>,
}

/// `AgentSessionInfo` (`agents.rs:226`). For Claude, `kind == "id"` and
/// `value` is the Claude session UUID.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentSessionInfo {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub agent: String,
    /// `"id"` or `"path"`.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub value: String,
}

/// `PaneInfo` subset (`panes.rs:447`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PaneInfo {
    pub pane_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_session: Option<AgentSessionInfo>,
}

/// `TabInfo` subset (`tabs.rs`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TabInfo {
    pub tab_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub label: String,
}

/// `WorkspaceInfo` subset (`workspaces.rs:55`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    #[serde(default)]
    pub label: String,
}

/// The `EventEnvelope` (`events.rs:362`) as delivered in `HERDR_PLUGIN_EVENT_JSON`.
/// We deliberately keep `event` as a raw string and read only `data.*` (
/// open question #6: the serialized form of `event` is not depended upon).
#[derive(Debug, Clone, Deserialize)]
pub struct EventEnvelope {
    #[serde(default)]
    pub event: Option<String>,
    pub data: EventData,
}

/// `EventData` (`events.rs:426`) — `#[serde(tag = "type")]`, snake_case. We only
/// model the fields we read: a `pane_id` (present on most variants) or a nested
/// `pane` (on `pane_created` / `pane_moved`). Unknown variants deserialize into
/// the tolerant fallback so parsing never fails on an event we do not handle.
#[derive(Debug, Clone, Deserialize)]
pub struct EventData {
    #[serde(rename = "type", default)]
    pub type_: Option<String>,
    #[serde(default)]
    pub pane_id: Option<String>,
    #[serde(default)]
    pub pane: Option<PaneInfo>,
}

impl EventData {
    /// Resolve the target pane id: `data.pane_id ?? data.pane.pane_id` (the
    /// `$HERDR_PANE_ID` fallback is applied by the caller).
    pub fn target_pane_id(&self) -> Option<String> {
        self.pane_id
            .clone()
            .or_else(|| self.pane.as_ref().map(|p| p.pane_id.clone()))
    }
}

impl EventEnvelope {
    /// Parse from the raw `HERDR_PLUGIN_EVENT_JSON` string.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_info_tolerates_unknown_fields() {
        let j = r#"{"pane_id":"w1:p1","workspace_id":"w1","tab_id":"t1",
            "agent":"claude","agent_status":"idle","cwd":"/x","focused":true,
            "revision":5,"terminal_id":"term-1","brand_new_field":123}"#;
        let a: AgentInfo = serde_json::from_str(j).unwrap();
        assert_eq!(a.pane_id, "w1:p1");
        assert_eq!(a.agent.as_deref(), Some("claude"));
        assert!(a.focused);
    }

    #[test]
    fn agent_session_id_kind() {
        let j = r#"{"source":"plugin:x","agent":"claude","kind":"id",
            "value":"00000000-0000-4000-8000-000000000000"}"#;
        let s: AgentSessionInfo = serde_json::from_str(j).unwrap();
        assert_eq!(s.kind, "id");
        assert_eq!(s.value, "00000000-0000-4000-8000-000000000000");
    }

    #[test]
    fn event_pane_focused_has_pane_id() {
        let j = r#"{"event":"pane.focused","data":{"type":"pane_focused",
            "pane_id":"w2:p3","workspace_id":"w2"}}"#;
        let e = EventEnvelope::parse(j).unwrap();
        assert_eq!(e.data.type_.as_deref(), Some("pane_focused"));
        assert_eq!(e.data.target_pane_id().as_deref(), Some("w2:p3"));
    }

    #[test]
    fn event_pane_created_nests_pane() {
        let j = r#"{"event":"pane.created","data":{"type":"pane_created",
            "pane":{"pane_id":"w9:p1","terminal_id":"t","workspace_id":"w9",
            "tab_id":"tab","focused":false,"agent":"claude","revision":1}}}"#;
        let e = EventEnvelope::parse(j).unwrap();
        assert_eq!(e.data.target_pane_id().as_deref(), Some("w9:p1"));
        assert_eq!(
            e.data.pane.as_ref().and_then(|p| p.agent.as_deref()),
            Some("claude")
        );
    }

    #[test]
    fn event_unknown_variant_still_parses_pane_id() {
        // A future event type we do not special-case: we still read pane_id.
        let j = r#"{"event":"pane.some_future","data":{"type":"pane_some_future",
            "pane_id":"w1:p1","workspace_id":"w1"}}"#;
        let e = EventEnvelope::parse(j).unwrap();
        assert_eq!(e.data.target_pane_id().as_deref(), Some("w1:p1"));
    }

    #[test]
    fn agent_list_wrapper_shape() {
        // The CLI prints {"id":..,"result":{"type":"agent_list","agents":[...]}}.
        let j = r#"{"id":"cli:agent:list","result":{"type":"agent_list",
            "agents":[{"pane_id":"w1:p1","terminal_id":"t","workspace_id":"w1",
            "tab_id":"t1","agent":"claude","agent_status":"idle","focused":false,
            "revision":1}]}}"#;
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        let agents: Vec<AgentInfo> = serde_json::from_value(v["result"]["agents"].clone()).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pane_id, "w1:p1");
    }
}
