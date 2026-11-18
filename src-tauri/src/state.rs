//! App-wide state fed by the herdr supervisor, and its UI projection.

use serde::Serialize;

use crate::agents::attention::{Attention, NeedsYouItem, should_notify};
use crate::herdr::client::HerdrClient;
use crate::herdr::model::{Agent, AgentStatus, ModelChange, SessionModel};
use crate::herdr::supervisor::SessionUpdate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Connection {
    Connecting,
    Connected,
    NotRunning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceView {
    pub id: String,
    pub label: String,
    pub agents: Vec<Agent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    pub connection: Connection,
    pub spaces: Vec<SpaceView>,
    pub needs_you: Vec<NeedsYouItem>,
}

/// A notification to post: an agent that just became blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub pane_id: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug)]
pub struct AppState {
    pub client: Option<HerdrClient>,
    pub model: SessionModel,
    pub attention: Attention,
    pub connection: Connection,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            client: None,
            model: SessionModel::default(),
            attention: Attention::default(),
            connection: Connection::Connecting,
        }
    }
}

impl AppState {
    pub fn to_ui(&self) -> UiState {
        let spaces = self
            .model
            .spaces
            .iter()
            .map(|sp| SpaceView {
                id: sp.id.clone(),
                label: sp.label.clone(),
                agents: self.model.agents.values().filter(|a| a.workspace_id == sp.id).cloned().collect(),
            })
            .collect();
        UiState { connection: self.connection, spaces, needs_you: self.attention.needs_you(&self.model) }
    }

    /// Take in a supervisor update and return the notifications it calls for.
    /// `Connecting` (a transient drop) keeps the last model on screen;
    /// `NotRunning` clears it, since herdr's agents are gone with the server.
    pub fn apply(&mut self, update: SessionUpdate) -> Vec<Notice> {
        match update {
            SessionUpdate::Connecting => {
                self.client = None;
                self.connection = Connection::Connecting;
                vec![]
            }
            SessionUpdate::NotRunning => {
                self.client = None;
                self.model = SessionModel::default();
                self.connection = Connection::NotRunning;
                vec![]
            }
            SessionUpdate::Connected { client, model, changes, live } => {
                self.client = Some(client);
                self.model = model;
                self.connection = Connection::Connected;
                changes.iter().filter(|c| should_notify(c, live)).filter_map(|c| self.notice(c)).collect()
            }
        }
    }

    fn notice(&self, change: &ModelChange) -> Option<Notice> {
        let ModelChange::AgentStatus { pane_id, .. } = change else { return None };
        let agent = self.model.agents.get(pane_id)?;
        let body = self
            .model
            .spaces
            .iter()
            .find(|s| s.id == agent.workspace_id)
            .map(|s| s.label.clone())
            .unwrap_or_default();
        let name = agent.title.as_deref().filter(|t| !t.is_empty()).unwrap_or("Agent");
        Some(Notice { pane_id: pane_id.clone(), title: format!("{name} needs you"), body })
    }
}

/// Dock badge for a UI state: the number of blocked items, none at 0.
pub fn badge_count(ui: &UiState) -> Option<i64> {
    let n = ui.needs_you.iter().filter(|i| i.status == AgentStatus::Blocked).count();
    (n > 0).then_some(n as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::model::Space;
    use std::path::Path;

    fn client() -> HerdrClient {
        // Never used to call: AppState only stores it.
        HerdrClient::unconnected(Path::new("/nonexistent"))
    }

    fn agent(pane: &str, ws: &str, title: Option<&str>, status: AgentStatus, seq: u64) -> Agent {
        Agent {
            pane_id: pane.into(),
            workspace_id: ws.into(),
            tab_id: "t".into(),
            agent: Some("claude".into()),
            title: title.map(str::to_string),
            status,
            state_change_seq: seq,
        }
    }

    fn model(agents: Vec<Agent>) -> SessionModel {
        let mut m = SessionModel::default();
        m.spaces.push(Space { id: "w2".into(), label: "beta".into(), cwd: None });
        m.spaces.push(Space { id: "w1".into(), label: "alpha".into(), cwd: None });
        for a in agents {
            m.agents.insert(a.pane_id.clone(), a);
        }
        m
    }

    fn connected(model: SessionModel, changes: Vec<ModelChange>, live: bool) -> SessionUpdate {
        SessionUpdate::Connected { client: client(), model, changes, live }
    }

    fn to_blocked(pane: &str, old: Option<AgentStatus>) -> ModelChange {
        ModelChange::AgentStatus { pane_id: pane.into(), old, new: AgentStatus::Blocked }
    }

    #[test]
    fn starts_connecting_and_empty() {
        let s = AppState::default();
        let ui = s.to_ui();
        assert_eq!(ui.connection, Connection::Connecting);
        assert!(ui.spaces.is_empty() && ui.needs_you.is_empty());
        assert!(s.client.is_none());
    }

    #[test]
    fn ui_state_serializes_camel_case_with_connection_strings() {
        let mut s = AppState::default();
        s.apply(connected(model(vec![agent("w1:p1", "w1", None, AgentStatus::Blocked, 1)]), vec![], false));
        let v = serde_json::to_value(s.to_ui()).unwrap();
        assert_eq!(v["connection"], "connected");
        assert_eq!(v["needsYou"][0]["paneId"], "w1:p1");
        assert_eq!(v["spaces"][1]["agents"][0]["stateChangeSeq"], 1);
        s.apply(SessionUpdate::NotRunning);
        assert_eq!(serde_json::to_value(s.to_ui()).unwrap()["connection"], "notRunning");
        s.apply(SessionUpdate::Connecting);
        assert_eq!(serde_json::to_value(s.to_ui()).unwrap()["connection"], "connecting");
    }

    #[test]
    fn spaces_follow_herdr_order_with_their_agents() {
        let mut s = AppState::default();
        s.apply(connected(
            model(vec![
                agent("w1:p1", "w1", None, AgentStatus::Idle, 1),
                agent("w2:p2", "w2", None, AgentStatus::Working, 1),
                agent("w1:p3", "w1", None, AgentStatus::Done, 1),
            ]),
            vec![],
            false,
        ));
        let ui = s.to_ui();
        let ids: Vec<_> = ui.spaces.iter().map(|sp| sp.id.as_str()).collect();
        assert_eq!(ids, vec!["w2", "w1"]);
        assert_eq!(ui.spaces[0].label, "beta");
        let w1: Vec<_> = ui.spaces[1].agents.iter().map(|a| a.pane_id.as_str()).collect();
        assert_eq!(w1, vec!["w1:p1", "w1:p3"]);
        assert!(s.client.is_some());
    }

    #[test]
    fn snapshot_with_blocked_agent_does_not_notify_but_live_transition_does() {
        let mut s = AppState::default();
        let m = model(vec![agent("w1:p1", "w1", Some("fix bug"), AgentStatus::Blocked, 6)]);
        assert!(s.apply(connected(m.clone(), vec![to_blocked("w1:p1", None)], false)).is_empty());

        let m2 = model(vec![
            agent("w1:p1", "w1", Some("fix bug"), AgentStatus::Blocked, 6),
            agent("w2:p2", "w2", None, AgentStatus::Blocked, 2),
        ]);
        let notices = s.apply(connected(m2, vec![to_blocked("w2:p2", Some(AgentStatus::Working))], true));
        assert_eq!(
            notices,
            vec![Notice { pane_id: "w2:p2".into(), title: "Agent needs you".into(), body: "beta".into() }]
        );
    }

    #[test]
    fn notice_uses_agent_title() {
        let mut s = AppState::default();
        let m = model(vec![agent("w1:p1", "w1", Some("fix bug"), AgentStatus::Blocked, 7)]);
        let notices = s.apply(connected(m, vec![to_blocked("w1:p1", Some(AgentStatus::Working))], true));
        assert_eq!(notices[0].title, "fix bug needs you");
        assert_eq!(notices[0].body, "alpha");
    }

    #[test]
    fn repeated_not_running_never_notifies_and_clears_session() {
        let mut s = AppState::default();
        s.apply(connected(model(vec![agent("w1:p1", "w1", None, AgentStatus::Blocked, 1)]), vec![], false));
        for _ in 0..3 {
            assert!(s.apply(SessionUpdate::NotRunning).is_empty());
        }
        assert!(s.client.is_none());
        let ui = s.to_ui();
        assert!(ui.spaces.is_empty() && ui.needs_you.is_empty());
        assert_eq!(badge_count(&ui), None);
    }

    #[test]
    fn connecting_keeps_last_model_but_drops_client() {
        let mut s = AppState::default();
        s.apply(connected(model(vec![agent("w1:p1", "w1", None, AgentStatus::Blocked, 1)]), vec![], false));
        assert!(s.apply(SessionUpdate::Connecting).is_empty());
        assert!(s.client.is_none());
        assert_eq!(s.to_ui().needs_you.len(), 1);
    }

    #[test]
    fn badge_counts_blocked_only_and_clears_at_zero() {
        let mut s = AppState::default();
        s.apply(connected(
            model(vec![
                agent("w1:p1", "w1", None, AgentStatus::Blocked, 1),
                agent("w1:p2", "w1", None, AgentStatus::Blocked, 2),
                agent("w2:p3", "w2", None, AgentStatus::Done, 3),
            ]),
            vec![],
            false,
        ));
        assert_eq!(badge_count(&s.to_ui()), Some(2));
        s.apply(connected(model(vec![agent("w2:p3", "w2", None, AgentStatus::Done, 3)]), vec![], true));
        assert_eq!(badge_count(&s.to_ui()), None);
    }
}
