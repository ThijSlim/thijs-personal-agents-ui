//! Keeps a `SessionModel` in sync with herdr and reconnects when it goes away.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::client::{HerdrClient, HerdrError, subscribe};
use super::model::{ModelChange, SessionModel};
use super::protocol::HerdrEvent;

const BACKOFF_START: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// Re-snapshot/resubscribe rounds before accepting a still-moving pane set.
const MAX_SYNC_ROUNDS: usize = 5;

/// Lifecycle subscriptions (from the schema's `Subscription` definition) that
/// can change Spaces, agents, or the set of panes.
const LIFECYCLE_EVENTS: &[&str] = &[
    "workspace.created",
    "workspace.updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "tab.closed",
    "pane.created",
    "pane.closed",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
];

#[derive(Debug, Clone)]
pub enum SessionUpdate {
    Connecting,
    NotRunning,
    Connected {
        client: HerdrClient,
        model: SessionModel,
        changes: Vec<ModelChange>,
        /// False for the first snapshot after each (re)connect: its changes are
        /// catch-up, not transitions observed as they happened.
        live: bool,
    },
}

/// Run forever (until `tx` is closed): connect, snapshot, subscribe, stream
/// updates; on failure report and retry with exponential backoff.
pub async fn run(path: PathBuf, tx: mpsc::Sender<SessionUpdate>) {
    let mut model = SessionModel::default();
    let mut backoff = BACKOFF_START;
    loop {
        let err = session(&path, &tx, &mut model, &mut backoff).await;
        let update = match err {
            Some(HerdrError::NotRunning) => SessionUpdate::NotRunning,
            Some(_) => SessionUpdate::Connecting,
            None => return, // receiver gone
        };
        if tx.send(update).await.is_err() {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// One connected session. Returns the error that ended it, or `None` if the
/// update receiver was dropped.
async fn session(
    path: &Path,
    tx: &mpsc::Sender<SessionUpdate>,
    model: &mut SessionModel,
    backoff: &mut Duration,
) -> Option<HerdrError> {
    let client = match HerdrClient::connect(path).await {
        Ok(c) => c,
        Err(e) => return Some(e),
    };
    let (new_model, mut panes, rx) = match sync(&client, path, None).await {
        Ok(s) => s,
        Err(e) => return Some(e),
    };
    let Some(mut events) = rx else { return Some(HerdrError::Protocol("no subscription".into())) };
    let changes = SessionModel::diff(model, &new_model);
    *model = new_model;
    *backoff = BACKOFF_START;
    let connected = |model: &SessionModel, changes, live| SessionUpdate::Connected {
        client: client.clone(),
        model: model.clone(),
        changes,
        live,
    };
    if tx.send(connected(model, changes, false)).await.is_err() {
        return None;
    }

    loop {
        let ev = tokio::select! {
            ev = events.recv() => match ev {
                Some(ev) => ev,
                None => return Some(HerdrError::Disconnected),
            },
            _ = tx.closed() => return None,
        };
        let changes = if needs_resync(&ev, &panes) {
            let (new_model, new_panes, rx) = match sync(&client, path, Some(&panes)).await {
                Ok(s) => s,
                Err(e) => return Some(e),
            };
            if let Some(rx) = rx {
                events = rx; // drops (and closes) the old subscription
            }
            panes = new_panes;
            let changes = SessionModel::diff(model, &new_model);
            *model = new_model;
            changes
        } else {
            model.apply_event(&ev)
        };
        if changes.is_empty() {
            continue;
        }
        if tx.send(connected(model, changes, true)).await.is_err() {
            return None;
        }
    }
}

/// Snapshot, and (re)subscribe whenever the snapshot's pane set differs from
/// what `subscribed` covers. Each new subscription is followed by a fresh
/// snapshot so nothing that happened before the subscription took effect is
/// missed. Returns the final model, the subscribed pane set, and the new
/// receiver if a subscription was opened.
async fn sync(
    client: &HerdrClient,
    path: &Path,
    subscribed: Option<&BTreeSet<String>>,
) -> Result<(SessionModel, BTreeSet<String>, Option<mpsc::Receiver<HerdrEvent>>), HerdrError> {
    let mut current = subscribed.cloned();
    let mut rx = None;
    let (mut model, mut panes) = snapshot(client).await?;
    for _ in 0..MAX_SYNC_ROUNDS {
        if current.as_ref() == Some(&panes) {
            break;
        }
        rx = Some(subscribe(path, subscriptions(&panes)).await?);
        current = Some(panes);
        (model, panes) = snapshot(client).await?;
    }
    let current = current.expect("at least one subscribe round ran");
    Ok((model, current, rx))
}

async fn snapshot(client: &HerdrClient) -> Result<(SessionModel, BTreeSet<String>), HerdrError> {
    let result = client.call("session.snapshot", json!({})).await?;
    let model = SessionModel::from_snapshot(&result).map_err(|e| HerdrError::Protocol(e.to_string()))?;
    let snap = result.get("snapshot").unwrap_or(&result);
    let panes = snap
        .get("panes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("pane_id").and_then(Value::as_str).map(str::to_string))
        .collect();
    Ok((model, panes))
}

fn subscriptions(panes: &BTreeSet<String>) -> Value {
    let lifecycle = LIFECYCLE_EVENTS.iter().map(|t| json!({ "type": t }));
    let status = panes.iter().map(|p| json!({ "type": "pane.agent_status_changed", "pane_id": p }));
    Value::Array(lifecycle.chain(status).collect())
}

/// Events that change the set of panes (or reveal one we don't watch) need a
/// fresh subscription and snapshot rather than an incremental apply.
fn needs_resync(ev: &HerdrEvent, panes: &BTreeSet<String>) -> bool {
    let unknown_pane = || {
        ev.data.get("pane_id").and_then(Value::as_str).is_none_or(|p| !panes.contains(p))
    };
    match ev.kind.replace('.', "_").as_str() {
        "pane_created" | "pane_closed" | "pane_moved" | "pane_exited" => true,
        "pane_agent_detected" | "pane_agent_status_changed" => unknown_pane(),
        _ => false,
    }
}
