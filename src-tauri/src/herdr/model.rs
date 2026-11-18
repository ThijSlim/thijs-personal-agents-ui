//! In-memory model of herdr's Spaces (workspaces) and agents, built from a
//! `session.snapshot` result and kept current by pushed events.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::protocol::{HerdrEvent, ProtocolError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    pub label: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub agent: Option<String>,
    pub title: Option<String>,
    pub status: AgentStatus,
    pub state_change_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelChange {
    SpacesChanged,
    AgentStatus { pane_id: String, old: Option<AgentStatus>, new: AgentStatus },
    AgentRemoved { pane_id: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionModel {
    pub spaces: Vec<Space>,
    pub agents: BTreeMap<String, Agent>,
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn req(v: &Value, key: &str) -> Result<String, ProtocolError> {
    s(v, key).ok_or_else(|| ProtocolError::Malformed(format!("missing field `{key}`")))
}

fn status(v: &Value) -> AgentStatus {
    v.get("agent_status")
        .cloned()
        .and_then(|x| serde_json::from_value(x).ok())
        .unwrap_or(AgentStatus::Unknown)
}

fn parse_agent(v: &Value) -> Result<Agent, ProtocolError> {
    Ok(Agent {
        pane_id: req(v, "pane_id")?,
        workspace_id: req(v, "workspace_id")?,
        tab_id: req(v, "tab_id")?,
        agent: s(v, "agent"),
        title: s(v, "title").or_else(|| s(v, "terminal_title_stripped")),
        status: status(v),
        state_change_seq: v.get("state_change_seq").and_then(Value::as_u64).unwrap_or(0),
    })
}

impl SessionModel {
    /// Build from a `session.snapshot` result. Accepts either the bare snapshot
    /// object or the result wrapper `{"snapshot": {...}}`. Workspaces carry no
    /// cwd in herdr, so a Space's cwd is its first pane's cwd.
    pub fn from_snapshot(result: &Value) -> Result<Self, ProtocolError> {
        let snap = result.get("snapshot").unwrap_or(result);
        let items = |key: &str| -> Result<&Vec<Value>, ProtocolError> {
            snap.get(key)
                .and_then(Value::as_array)
                .ok_or_else(|| ProtocolError::Malformed(format!("snapshot missing `{key}`")))
        };
        let panes = items("panes")?;
        let spaces = items("workspaces")?
            .iter()
            .map(|w| {
                let id = req(w, "workspace_id")?;
                let cwd = panes
                    .iter()
                    .find(|p| s(p, "workspace_id").as_deref() == Some(id.as_str()))
                    .and_then(|p| s(p, "cwd"));
                Ok(Space { id, label: s(w, "label").unwrap_or_default(), cwd })
            })
            .collect::<Result<Vec<_>, ProtocolError>>()?;
        let agents = items("agents")?
            .iter()
            .map(|a| parse_agent(a).map(|a| (a.pane_id.clone(), a)))
            .collect::<Result<_, _>>()?;
        Ok(Self { spaces, agents })
    }

    /// Apply a pushed event, returning what changed. Unused events return an empty vec.
    pub fn apply_event(&mut self, ev: &HerdrEvent) -> Vec<ModelChange> {
        let d = &ev.data;
        match ev.kind.replace('.', "_").as_str() {
            "pane_agent_status_changed" => {
                let Some(pane_id) = s(d, "pane_id") else { return vec![] };
                let new = status(d);
                if let Some(a) = self.agents.get_mut(&pane_id) {
                    let old = a.status;
                    a.status = new;
                    if s(d, "agent").is_some() {
                        a.agent = s(d, "agent");
                    }
                    if s(d, "title").is_some() {
                        a.title = s(d, "title");
                    }
                    if old == new {
                        return vec![];
                    }
                    vec![ModelChange::AgentStatus { pane_id, old: Some(old), new }]
                } else if s(d, "agent").is_some() {
                    // Agent first seen via event (not in snapshot): tab id is unknown.
                    let agent = Agent {
                        pane_id: pane_id.clone(),
                        workspace_id: s(d, "workspace_id").unwrap_or_default(),
                        tab_id: String::new(),
                        agent: s(d, "agent"),
                        title: s(d, "title"),
                        status: new,
                        state_change_seq: 0,
                    };
                    self.agents.insert(pane_id.clone(), agent);
                    vec![ModelChange::AgentStatus { pane_id, old: None, new }]
                } else {
                    vec![]
                }
            }
            "workspace_moved" | "workspace_reordered" => {
                let Some(list) = d.get("workspaces").and_then(Value::as_array) else { return vec![] };
                let old_spaces = std::mem::take(&mut self.spaces);
                self.spaces = list
                    .iter()
                    .filter_map(|w| {
                        let id = s(w, "workspace_id")?;
                        let cwd = old_spaces.iter().find(|o| o.id == id).and_then(|o| o.cwd.clone());
                        Some(Space { id, label: s(w, "label").unwrap_or_default(), cwd })
                    })
                    .collect();
                if self.spaces == old_spaces { vec![] } else { vec![ModelChange::SpacesChanged] }
            }
            "workspace_created" => {
                let Some(w) = d.get("workspace") else { return vec![] };
                let Some(id) = s(w, "workspace_id") else { return vec![] };
                if self.spaces.iter().any(|sp| sp.id == id) {
                    return vec![];
                }
                self.spaces.push(Space { id, label: s(w, "label").unwrap_or_default(), cwd: None });
                vec![ModelChange::SpacesChanged]
            }
            "workspace_renamed" => {
                let (Some(id), Some(label)) = (s(d, "workspace_id"), s(d, "label")) else { return vec![] };
                match self.spaces.iter_mut().find(|sp| sp.id == id) {
                    Some(sp) if sp.label != label => {
                        sp.label = label;
                        vec![ModelChange::SpacesChanged]
                    }
                    _ => vec![],
                }
            }
            "workspace_closed" => {
                let Some(id) = s(d, "workspace_id") else { return vec![] };
                let mut changes = vec![];
                let before = self.spaces.len();
                self.spaces.retain(|sp| sp.id != id);
                if self.spaces.len() != before {
                    changes.push(ModelChange::SpacesChanged);
                }
                let gone: Vec<String> = self
                    .agents
                    .values()
                    .filter(|a| a.workspace_id == id)
                    .map(|a| a.pane_id.clone())
                    .collect();
                for pane_id in gone {
                    self.agents.remove(&pane_id);
                    changes.push(ModelChange::AgentRemoved { pane_id });
                }
                changes
            }
            "pane_closed" => {
                let Some(pane_id) = s(d, "pane_id") else { return vec![] };
                if self.agents.remove(&pane_id).is_some() {
                    vec![ModelChange::AgentRemoved { pane_id }]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    /// Changes needed to get from `old` to `new` (e.g. after a reconnect re-snapshot).
    pub fn diff(old: &SessionModel, new: &SessionModel) -> Vec<ModelChange> {
        let mut changes = vec![];
        if old.spaces != new.spaces {
            changes.push(ModelChange::SpacesChanged);
        }
        for (pane_id, a) in &new.agents {
            let prev = old.agents.get(pane_id).map(|p| p.status);
            if prev != Some(a.status) {
                changes.push(ModelChange::AgentStatus { pane_id: pane_id.clone(), old: prev, new: a.status });
            }
        }
        for pane_id in old.agents.keys().filter(|k| !new.agents.contains_key(*k)) {
            changes.push(ModelChange::AgentRemoved { pane_id: pane_id.clone() });
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::protocol::{Incoming, parse_line};
    use serde_json::json;

    fn snapshot_model() -> SessionModel {
        let v: Value = serde_json::from_str(include_str!("../../tests/fixtures/snapshot.json")).unwrap();
        SessionModel::from_snapshot(&v["result"]).unwrap()
    }

    fn fixture_events() -> Vec<HerdrEvent> {
        include_str!("../../tests/fixtures/events.ndjson")
            .lines()
            .filter_map(|l| match parse_line(l).unwrap() {
                Incoming::Event(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    fn event(kind: &str, data: Value) -> HerdrEvent {
        HerdrEvent { kind: kind.into(), data }
    }

    fn agent(pane: &str, ws: &str, status: AgentStatus) -> Agent {
        Agent {
            pane_id: pane.into(),
            workspace_id: ws.into(),
            tab_id: "t".into(),
            agent: Some("claude".into()),
            title: None,
            status,
            state_change_seq: 1,
        }
    }

    #[test]
    fn snapshot_builds_spaces_in_order_and_agents() {
        let m = snapshot_model();
        assert_eq!(m.spaces.len(), 3);
        assert_eq!(m.spaces[0].label, "alpha");
        assert_eq!(m.spaces[0].cwd.as_deref(), Some("/private/tmp/au-a"));
        let a = &m.agents["w1:p1"];
        assert_eq!(a.status, AgentStatus::Blocked);
        assert_eq!(a.agent.as_deref(), Some("claude"));
        assert_eq!(a.workspace_id, "w1");
        assert_eq!(a.state_change_seq, 6);
    }

    #[test]
    fn unknown_status_maps_to_unknown() {
        let st: AgentStatus = serde_json::from_str("\"weird\"").unwrap();
        assert_eq!(st, AgentStatus::Unknown);
        assert_eq!(serde_json::to_string(&AgentStatus::Blocked).unwrap(), "\"blocked\"");
    }

    #[test]
    fn agent_serializes_camel_case() {
        let v = serde_json::to_value(agent("p", "w", AgentStatus::Idle)).unwrap();
        assert_eq!(v["paneId"], "p");
        assert_eq!(v["stateChangeSeq"], 1);
        assert_eq!(v["status"], "idle");
    }

    #[test]
    fn status_event_yields_change_with_old_and_new() {
        let mut m = snapshot_model();
        let changes: Vec<_> = fixture_events()
            .iter()
            .filter(|e| e.kind == "pane.agent_status_changed")
            .flat_map(|e| m.apply_event(e))
            .collect();
        let pane_id = "w1:p1".to_string();
        // Fixture: blocked(snapshot) -> working -> blocked -> working -> idle.
        assert!(changes.contains(&ModelChange::AgentStatus {
            pane_id: pane_id.clone(),
            old: Some(AgentStatus::Working),
            new: AgentStatus::Blocked
        }));
        assert_eq!(m.agents["w1:p1"].status, AgentStatus::Idle);
    }

    #[test]
    fn workspace_moved_reorders_spaces() {
        let mut m = snapshot_model();
        let moved = fixture_events().into_iter().find(|e| e.kind == "workspace_moved").unwrap();
        let expected: Vec<String> = moved.data["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["workspace_id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(m.apply_event(&moved), vec![ModelChange::SpacesChanged]);
        let ids: Vec<String> = m.spaces.iter().map(|s| s.id.clone()).collect();
        assert_eq!(ids, expected);
        assert_eq!(ids[0], "w4");
        // cwd survives the reorder for known spaces
        assert_eq!(m.spaces[1].cwd.as_deref(), Some("/private/tmp/au-a"));
    }

    #[test]
    fn workspace_closed_removes_space_and_its_agents() {
        let mut m = snapshot_model();
        let changes = m.apply_event(&event("workspace_closed", json!({"type":"workspace_closed","workspace_id":"w1"})));
        assert!(m.spaces.iter().all(|s| s.id != "w1"));
        assert!(m.agents.is_empty());
        assert_eq!(
            changes,
            vec![ModelChange::SpacesChanged, ModelChange::AgentRemoved { pane_id: "w1:p1".into() }]
        );
    }

    #[test]
    fn pane_closed_removes_agent() {
        let mut m = snapshot_model();
        let changes = m.apply_event(&event("pane_closed", json!({"pane_id":"w1:p1","workspace_id":"w1"})));
        assert_eq!(changes, vec![ModelChange::AgentRemoved { pane_id: "w1:p1".into() }]);
        assert!(m.apply_event(&event("pane_closed", json!({"pane_id":"w9:p9"}))).is_empty());
    }

    #[test]
    fn captured_events_apply_without_panic_and_unused_are_noops() {
        let mut m = snapshot_model();
        for e in fixture_events() {
            m.apply_event(&e);
        }
        assert!(m.spaces.iter().all(|s| s.id != "w4"));
        assert!(m.apply_event(&event("pane_created", json!({}))).is_empty());
    }

    #[test]
    fn diff_reports_status_changes_and_removals() {
        let mut old = SessionModel::default();
        old.agents.insert("a".into(), agent("a", "w", AgentStatus::Working));
        old.agents.insert("b".into(), agent("b", "w", AgentStatus::Idle));
        let mut new = old.clone();
        new.agents.get_mut("a").unwrap().status = AgentStatus::Blocked;
        new.agents.remove("b");
        new.agents.insert("c".into(), agent("c", "w", AgentStatus::Idle));
        new.spaces.push(Space { id: "w".into(), label: "x".into(), cwd: None });

        let changes = SessionModel::diff(&old, &new);
        assert_eq!(changes.len(), 4);
        assert!(changes.contains(&ModelChange::SpacesChanged));
        assert!(changes.contains(&ModelChange::AgentStatus {
            pane_id: "a".into(),
            old: Some(AgentStatus::Working),
            new: AgentStatus::Blocked
        }));
        assert!(changes.contains(&ModelChange::AgentStatus {
            pane_id: "c".into(),
            old: None,
            new: AgentStatus::Idle
        }));
        assert!(changes.contains(&ModelChange::AgentRemoved { pane_id: "b".into() }));
        assert!(SessionModel::diff(&old, &old.clone()).is_empty());
    }
}
