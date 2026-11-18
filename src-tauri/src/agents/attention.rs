//! What needs the user's attention: blocked agents, unseen done agents, and
//! which status changes deserve a notification.

use std::collections::HashSet;

use serde::Serialize;

use crate::herdr::model::{AgentStatus, ModelChange, SessionModel};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeedsYouItem {
    pub pane_id: String,
    pub workspace_id: String,
    pub space_label: String,
    pub title: Option<String>,
    pub status: AgentStatus,
}

#[derive(Debug, Clone, Default)]
pub struct Attention {
    pub seen: HashSet<(String, u64)>,
}

impl Attention {
    /// The user has looked at this pane in this state; a later state change
    /// (higher seq) makes it show again.
    pub fn mark_seen(&mut self, pane_id: &str, state_change_seq: u64) {
        self.seen.insert((pane_id.to_string(), state_change_seq));
    }

    /// Forget seen entries that no longer describe a live agent state: the
    /// pane is gone, or the entry's seq is ahead of the agent's (herdr
    /// restarted and counts from low again). Without this, a reused pane id
    /// with a matching seq would hide a new `done` agent.
    pub fn prune(&mut self, model: &SessionModel) {
        self.seen
            .retain(|(pane, seq)| model.agents.get(pane).is_some_and(|a| *seq <= a.state_change_seq));
    }

    /// Blocked agents, then done agents not yet seen; each group newest
    /// state change first.
    pub fn needs_you(&self, model: &SessionModel) -> Vec<NeedsYouItem> {
        let group = |status: AgentStatus, skip_seen: bool| {
            let mut agents: Vec<_> = model
                .agents
                .values()
                .filter(|a| a.status == status)
                .filter(|a| !(skip_seen && self.seen.contains(&(a.pane_id.clone(), a.state_change_seq))))
                .collect();
            agents.sort_by_key(|a| std::cmp::Reverse(a.state_change_seq));
            agents.into_iter().map(|a| NeedsYouItem {
                pane_id: a.pane_id.clone(),
                workspace_id: a.workspace_id.clone(),
                space_label: model
                    .spaces
                    .iter()
                    .find(|s| s.id == a.workspace_id)
                    .map(|s| s.label.clone())
                    .unwrap_or_default(),
                title: a.title.clone(),
                status: a.status,
            })
        };
        group(AgentStatus::Blocked, false).chain(group(AgentStatus::Done, true)).collect()
    }
}

/// Notify only for a transition into blocked observed live (not catch-up
/// from a snapshot after (re)connecting).
pub fn should_notify(change: &ModelChange, live: bool) -> bool {
    live && matches!(
        change,
        ModelChange::AgentStatus { old, new: AgentStatus::Blocked, .. } if *old != Some(AgentStatus::Blocked)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::model::{Agent, Space};

    fn agent(pane: &str, ws: &str, status: AgentStatus, seq: u64) -> Agent {
        Agent {
            pane_id: pane.into(),
            workspace_id: ws.into(),
            tab_id: "t".into(),
            agent: Some("claude".into()),
            title: Some(format!("title {pane}")),
            status,
            state_change_seq: seq,
        }
    }

    fn model(agents: Vec<Agent>) -> SessionModel {
        let mut m = SessionModel::default();
        m.spaces.push(Space { id: "w1".into(), label: "alpha".into(), cwd: None });
        m.spaces.push(Space { id: "w2".into(), label: "beta".into(), cwd: None });
        for a in agents {
            m.agents.insert(a.pane_id.clone(), a);
        }
        m
    }

    fn panes(items: &[NeedsYouItem]) -> Vec<&str> {
        items.iter().map(|i| i.pane_id.as_str()).collect()
    }

    fn status(pane: &str, old: Option<AgentStatus>, new: AgentStatus) -> ModelChange {
        ModelChange::AgentStatus { pane_id: pane.into(), old, new }
    }

    #[test]
    fn needs_you_orders_blocked_then_unseen_done() {
        let m = model(vec![
            agent("w1:p1", "w1", AgentStatus::Done, 9),
            agent("w1:p2", "w1", AgentStatus::Blocked, 3),
            agent("w2:p3", "w2", AgentStatus::Blocked, 8),
            agent("w2:p4", "w2", AgentStatus::Done, 2),
        ]);
        let items = Attention::default().needs_you(&m);
        assert_eq!(panes(&items), vec!["w2:p3", "w1:p2", "w1:p1", "w2:p4"]);
        assert_eq!(items[0].space_label, "beta");
        assert_eq!(items[0].workspace_id, "w2");
        assert_eq!(items[0].title.as_deref(), Some("title w2:p3"));
        assert_eq!(items[0].status, AgentStatus::Blocked);
        assert_eq!(items[2].space_label, "alpha");
    }

    #[test]
    fn seen_done_is_hidden_until_new_state_change() {
        let mut att = Attention::default();
        att.mark_seen("w1:p1", 7);
        let m = model(vec![agent("w1:p1", "w1", AgentStatus::Done, 7)]);
        assert!(att.needs_you(&m).is_empty());
        let m = model(vec![agent("w1:p1", "w1", AgentStatus::Done, 9)]);
        assert_eq!(panes(&att.needs_you(&m)), vec!["w1:p1"]);
    }

    #[test]
    fn notify_only_on_live_transition_into_blocked() {
        let wb = status("p", Some(AgentStatus::Working), AgentStatus::Blocked);
        assert!(should_notify(&wb, true));
        assert!(!should_notify(&wb, false));
        assert!(!should_notify(&status("p", Some(AgentStatus::Blocked), AgentStatus::Blocked), true));
        assert!(should_notify(&status("p", None, AgentStatus::Blocked), true));
        assert!(!should_notify(&status("p", Some(AgentStatus::Working), AgentStatus::Done), true));
        assert!(!should_notify(&ModelChange::AgentRemoved { pane_id: "p".into() }, true));
        assert!(!should_notify(&ModelChange::SpacesChanged, true));
    }

    #[test]
    fn idle_and_working_never_in_needs_you() {
        let m = model(vec![
            agent("w1:p1", "w1", AgentStatus::Idle, 1),
            agent("w1:p2", "w1", AgentStatus::Working, 2),
            agent("w1:p3", "w1", AgentStatus::Unknown, 3),
        ]);
        assert!(Attention::default().needs_you(&m).is_empty());
    }

    #[test]
    fn needs_you_item_serializes_camel_case() {
        let m = model(vec![agent("w1:p1", "w1", AgentStatus::Blocked, 1)]);
        let v = serde_json::to_value(&Attention::default().needs_you(&m)[0]).unwrap();
        assert_eq!(v["paneId"], "w1:p1");
        assert_eq!(v["workspaceId"], "w1");
        assert_eq!(v["spaceLabel"], "alpha");
        assert_eq!(v["status"], "blocked");
    }
}
