mod fake_herdr;

use std::time::Duration;

use agents_lib::herdr::model::{AgentStatus, ModelChange};
use agents_lib::herdr::supervisor::{SessionUpdate, run, run_with_timeout};
use fake_herdr::{FakeHerdr, fixture_snapshot};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::timeout;

const T: Duration = Duration::from_secs(2);

/// Fixture snapshot with agent w1:p1 (in both `agents` and `panes`) set to `status`.
fn snapshot_with_status(status: &str) -> Value {
    let mut v = fixture_snapshot();
    for key in ["agents", "panes"] {
        for p in v["snapshot"][key].as_array_mut().unwrap() {
            if p["pane_id"] == "w1:p1" {
                p["agent_status"] = json!(status);
            }
        }
    }
    v
}

async fn next(rx: &mut mpsc::Receiver<SessionUpdate>) -> SessionUpdate {
    timeout(T, rx.recv()).await.expect("update within 2s").expect("supervisor alive")
}

fn status_sub_panes(req: &Value) -> Vec<String> {
    req["params"]["subscriptions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["type"] == "pane.agent_status_changed")
        .map(|s| s["pane_id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn supervisor_reconnects_after_restart() {
    let (fake, path) = FakeHerdr::start();
    fake.on("session.snapshot", snapshot_with_status("working"));
    let (tx, mut rx) = mpsc::channel(32);
    let sup = tokio::spawn(run(path, tx));

    match next(&mut rx).await {
        SessionUpdate::Connected { live, model, .. } => {
            assert!(!live);
            assert_eq!(model.agents["w1:p1"].status, AgentStatus::Working);
        }
        other => panic!("expected Connected, got {other:?}"),
    }

    fake.stop();
    match next(&mut rx).await {
        SessionUpdate::NotRunning | SessionUpdate::Connecting => {}
        other => panic!("expected NotRunning/Connecting, got {other:?}"),
    }

    fake.on("session.snapshot", snapshot_with_status("blocked"));
    fake.restart();
    let (live, changes) = loop {
        match next(&mut rx).await {
            SessionUpdate::Connected { live, changes, .. } => break (live, changes),
            SessionUpdate::NotRunning | SessionUpdate::Connecting => continue,
        }
    };
    assert!(!live, "first snapshot after a reconnect is not live");
    assert!(
        changes.contains(&ModelChange::AgentStatus {
            pane_id: "w1:p1".into(),
            old: Some(AgentStatus::Working),
            new: AgentStatus::Blocked,
        }),
        "changes: {changes:?}"
    );
    sup.abort();
}

#[tokio::test]
async fn subscribes_to_lifecycle_and_every_pane_status() {
    let (fake, path) = FakeHerdr::start();
    fake.on("session.snapshot", fixture_snapshot());
    let (tx, mut rx) = mpsc::channel(32);
    let sup = tokio::spawn(run(path, tx));
    assert!(matches!(next(&mut rx).await, SessionUpdate::Connected { live: false, .. }));

    let req = fake.wait_for_request(|r| r["method"] == "events.subscribe").await;
    let types: Vec<&str> = req["params"]["subscriptions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["type"].as_str().unwrap())
        .collect();
    for t in ["workspace.created", "workspace.moved", "workspace.closed", "tab.created", "tab.closed", "pane.created", "pane.closed", "pane.moved", "pane.exited", "pane.agent_detected"] {
        assert!(types.contains(&t), "missing {t} in {types:?}");
    }
    let mut panes = status_sub_panes(&req);
    panes.sort();
    assert_eq!(panes, vec!["w1:p1", "w2:p1", "w3:p1"]);

    // A live status event is applied and reported with live: true.
    fake.push_event(json!({"event": "pane.agent_status_changed", "data": {"pane_id": "w1:p1", "workspace_id": "w1", "agent_status": "working"}}));
    match next(&mut rx).await {
        SessionUpdate::Connected { live, changes, .. } => {
            assert!(live);
            assert_eq!(changes, vec![ModelChange::AgentStatus {
                pane_id: "w1:p1".into(),
                old: Some(AgentStatus::Blocked),
                new: AgentStatus::Working,
            }]);
        }
        other => panic!("expected Connected, got {other:?}"),
    }
    sup.abort();
}

#[tokio::test]
async fn pane_created_resubscribes_with_new_pane() {
    let (fake, path) = FakeHerdr::start();
    fake.on("session.snapshot", fixture_snapshot());
    let (tx, mut rx) = mpsc::channel(32);
    let sup = tokio::spawn(run(path, tx));
    assert!(matches!(next(&mut rx).await, SessionUpdate::Connected { live: false, .. }));

    // herdr now has a new pane w1:p2 running an idle agent.
    let mut snap = fixture_snapshot();
    let mut pane = snap["snapshot"]["panes"][0].clone();
    pane["pane_id"] = json!("w1:p2");
    pane["agent_status"] = json!("idle");
    snap["snapshot"]["panes"].as_array_mut().unwrap().push(pane.clone());
    snap["snapshot"]["agents"].as_array_mut().unwrap().push(pane.clone());
    fake.on("session.snapshot", snap);
    fake.push_event(json!({"event": "pane_created", "data": {"type": "pane_created", "pane": pane}}));

    let req = timeout(
        T,
        fake.wait_for_request(|r| {
            r["method"] == "events.subscribe" && status_sub_panes(r).contains(&"w1:p2".to_string())
        }),
    )
    .await
    .expect("resubscribe including the new pane");
    assert!(status_sub_panes(&req).contains(&"w1:p1".to_string()));

    match next(&mut rx).await {
        SessionUpdate::Connected { live, changes, model, .. } => {
            assert!(live, "a resubscribe is not a reconnect");
            assert!(model.agents.contains_key("w1:p2"));
            assert!(changes.contains(&ModelChange::AgentStatus {
                pane_id: "w1:p2".into(),
                old: None,
                new: AgentStatus::Idle,
            }), "changes: {changes:?}");
        }
        other => panic!("expected Connected, got {other:?}"),
    }
    sup.abort();
}

#[tokio::test]
async fn supervisor_retries_when_herdr_never_replies() {
    let (fake, path) = FakeHerdr::start();
    fake.on_never("session.snapshot");
    let (tx, mut rx) = mpsc::channel(32);
    let sup = tokio::spawn(run_with_timeout(path, tx, Duration::from_millis(200)));
    assert!(matches!(next(&mut rx).await, SessionUpdate::Connecting), "hung herdr must not block the supervisor");

    fake.on("session.snapshot", fixture_snapshot());
    let connected = loop {
        match next(&mut rx).await {
            SessionUpdate::Connected { live, .. } => break live,
            _ => continue,
        }
    };
    assert!(!connected);
    sup.abort();
}

#[tokio::test]
async fn workspace_created_resubscribes_with_root_pane() {
    let (fake, path) = FakeHerdr::start();
    fake.on("session.snapshot", fixture_snapshot());
    let (tx, mut rx) = mpsc::channel(32);
    let sup = tokio::spawn(run(path, tx));
    assert!(matches!(next(&mut rx).await, SessionUpdate::Connected { live: false, .. }));

    // herdr now has workspace w4 with root pane w4:p1 (not an agent).
    let mut snap = fixture_snapshot();
    let mut ws = snap["snapshot"]["workspaces"][0].clone();
    ws["workspace_id"] = json!("w4");
    ws["label"] = json!("gamma");
    let mut pane = snap["snapshot"]["panes"][1].clone();
    pane["pane_id"] = json!("w4:p1");
    pane["workspace_id"] = json!("w4");
    pane["tab_id"] = json!("w4:t1");
    snap["snapshot"]["workspaces"].as_array_mut().unwrap().push(ws.clone());
    snap["snapshot"]["panes"].as_array_mut().unwrap().push(pane.clone());
    fake.on("session.snapshot", snap);
    fake.push_event(json!({"event": "workspace_created", "data": {
        "type": "workspace_created", "workspace": ws,
        "tab": {"tab_id": "w4:t1", "workspace_id": "w4"}, "root_pane": pane
    }}));

    let req = fake
        .wait_for_request(|r| {
            r["method"] == "events.subscribe" && status_sub_panes(r).contains(&"w4:p1".to_string())
        })
        .await;
    assert!(status_sub_panes(&req).contains(&"w1:p1".to_string()));
    match next(&mut rx).await {
        SessionUpdate::Connected { live, changes, model, .. } => {
            assert!(live);
            assert!(model.spaces.iter().any(|s| s.id == "w4"));
            assert!(changes.contains(&ModelChange::SpacesChanged), "changes: {changes:?}");
        }
        other => panic!("expected Connected, got {other:?}"),
    }
    sup.abort();
}

/// Smoke test: the real herdr accepts our subscription list. Use a named dev
/// session, never the default one (see `real_herdr_ping` in client_test.rs).
#[tokio::test]
#[ignore]
async fn real_herdr_supervisor_connects() {
    assert!(
        std::env::var_os("HERDR_SOCKET_PATH").is_some(),
        "set HERDR_SOCKET_PATH to a named dev session socket, not the default session"
    );
    let path = agents_lib::herdr::client::default_socket_path();
    let (tx, mut rx) = mpsc::channel(32);
    let sup = tokio::spawn(run(path, tx));
    match next(&mut rx).await {
        SessionUpdate::Connected { live, model, .. } => {
            assert!(!live);
            println!("real herdr: {} spaces, {} agents", model.spaces.len(), model.agents.len());
        }
        other => panic!("expected Connected, got {other:?}"),
    }
    sup.abort();
}
