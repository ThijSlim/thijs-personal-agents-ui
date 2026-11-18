mod fake_herdr;

use std::time::Duration;

use agents_lib::herdr::client::{HerdrClient, HerdrError, default_socket_path, subscribe};
use fake_herdr::FakeHerdr;
use serde_json::json;
use tokio::time::timeout;

const T: Duration = Duration::from_secs(2);

#[tokio::test]
async fn call_returns_result_and_maps_error() {
    let (fake, path) = FakeHerdr::start();
    fake.on_error("pane.get", "not_found", "no such pane");
    let client = timeout(T, HerdrClient::connect(&path)).await.unwrap().unwrap();

    let pong = timeout(T, client.call("ping", json!({}))).await.unwrap().unwrap();
    assert_eq!(pong, json!({"type": "pong"}));

    let err = timeout(T, client.call("pane.get", json!({"pane_id": "w9:p9"}))).await.unwrap().unwrap_err();
    assert!(
        matches!(&err, HerdrError::Api { code, message } if code == "not_found" && message == "no such pane"),
        "got {err:?}"
    );
    let req = fake.wait_for_request(|r| r["method"] == "pane.get").await;
    assert_eq!(req["params"], json!({"pane_id": "w9:p9"}));
}

#[tokio::test]
async fn concurrent_calls_are_correlated() {
    let (fake, path) = FakeHerdr::start();
    let gate = fake.on_deferred("first", json!({"n": 1}));
    fake.on("second", json!({"n": 2}));
    let client = timeout(T, HerdrClient::connect(&path)).await.unwrap().unwrap();

    let c1 = client.clone();
    let first = tokio::spawn(async move { c1.call("first", json!({})).await });
    fake.wait_for_request(|r| r["method"] == "first").await;
    // "second" is answered while "first" is still held: responses arrive in reverse order.
    let second = timeout(T, client.call("second", json!({}))).await.unwrap().unwrap();
    assert_eq!(second, json!({"n": 2}));
    assert!(!first.is_finished());
    gate.open();
    let first = timeout(T, first).await.unwrap().unwrap().unwrap();
    assert_eq!(first, json!({"n": 1}));
}

#[tokio::test]
async fn connect_to_missing_socket_is_not_running() {
    let path = std::env::temp_dir().join(format!("no-herdr-{}.sock", std::process::id()));
    let err = HerdrClient::connect(&path).await.unwrap_err();
    assert!(matches!(err, HerdrError::NotRunning), "got {err:?}");
    let err = subscribe(&path, json!([])).await.unwrap_err();
    assert!(matches!(err, HerdrError::NotRunning), "got {err:?}");
}

#[tokio::test]
async fn call_after_server_stops_is_disconnected() {
    let (fake, path) = FakeHerdr::start();
    let client = timeout(T, HerdrClient::connect(&path)).await.unwrap().unwrap();
    let gate = fake.on_deferred("slow", json!({}));
    let c = client.clone();
    let pending = tokio::spawn(async move { c.call("slow", json!({})).await });
    fake.wait_for_request(|r| r["method"] == "slow").await;
    fake.stop();
    let err = timeout(T, pending).await.unwrap().unwrap().unwrap_err();
    assert!(matches!(err, HerdrError::Disconnected), "got {err:?}");
    drop(gate);
}

#[tokio::test]
async fn subscribe_receives_pushed_events() {
    let (fake, path) = FakeHerdr::start();
    let subs = json!([{"type": "pane.created"}, {"type": "pane.agent_status_changed", "pane_id": "w1:p1"}]);
    let mut rx = timeout(T, subscribe(&path, subs.clone())).await.unwrap().unwrap();

    let req = fake.wait_for_request(|r| r["method"] == "events.subscribe").await;
    assert_eq!(req["params"], json!({"subscriptions": subs}));

    assert_eq!(fake.push_event(json!({"event": "pane.agent_status_changed", "data": {"pane_id": "w1:p1", "agent_status": "blocked"}})), 1);
    let ev = timeout(T, rx.recv()).await.unwrap().unwrap();
    assert_eq!(ev.kind, "pane.agent_status_changed");
    assert_eq!(ev.data["agent_status"], "blocked");

    fake.stop();
    assert!(timeout(T, rx.recv()).await.unwrap().is_none(), "receiver closes with the socket");
}

/// Smoke test against a real herdr. NEVER point this at your default session:
///   HERDR_SESSION=agents-ui-dev herdr server &
///   HERDR_SOCKET_PATH=~/.config/herdr/sessions/agents-ui-dev/herdr.sock \
///     cargo test --manifest-path src-tauri/Cargo.toml --test client_test -- --ignored
///   HERDR_SESSION=agents-ui-dev herdr server stop
#[tokio::test]
#[ignore]
async fn real_herdr_ping() {
    assert!(
        std::env::var_os("HERDR_SOCKET_PATH").is_some(),
        "set HERDR_SOCKET_PATH to a named dev session socket, not the default session"
    );
    let client = timeout(T, HerdrClient::connect(&default_socket_path())).await.unwrap().unwrap();
    let pong = timeout(T, client.call("ping", json!({}))).await.unwrap().unwrap();
    println!("real herdr ping -> {pong}");
    assert_eq!(pong["type"], "pong");
}
