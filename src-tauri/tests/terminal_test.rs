mod fake_herdr;

use std::time::Duration;

use agents_lib::commands::{Screen, input, screen};
use agents_lib::herdr::client::HerdrClient;
use fake_herdr::FakeHerdr;
use serde_json::{Value, json};
use tokio::time::timeout;

const T: Duration = Duration::from_secs(2);

async fn client(path: &std::path::Path) -> HerdrClient {
    timeout(T, HerdrClient::connect(path)).await.unwrap().unwrap()
}

fn read_result(text: &str) -> Value {
    json!({ "type": "pane_read", "read": {
        "pane_id": "w1:p1", "workspace_id": "w1", "tab_id": "w1:t1",
        "source": "visible", "format": "ansi", "text": text, "revision": 1, "truncated": false } })
}

#[tokio::test]
async fn screen_reads_visible_with_ansi_kept() {
    let (fake, path) = FakeHerdr::start();
    fake.on("pane.read", read_result("\x1b[31mred\x1b[0m"));
    let c = client(&path).await;
    let s = timeout(T, screen(&c, "w1:p1")).await.unwrap().unwrap();
    assert_eq!(s, Screen { text: "\x1b[31mred\x1b[0m".into(), ended: false });
    let req = fake.wait_for_request(|r| r["method"] == "pane.read").await;
    assert_eq!(
        req["params"],
        json!({ "pane_id": "w1:p1", "source": "visible", "format": "ansi", "strip_ansi": false })
    );
}

#[tokio::test]
async fn screen_not_found_is_ended_and_other_errors_surface() {
    let (fake, path) = FakeHerdr::start();
    fake.on_error("pane.read", "pane_not_found", "gone");
    let c = client(&path).await;
    let s = timeout(T, screen(&c, "w1:p1")).await.unwrap().unwrap();
    assert_eq!(s, Screen { text: String::new(), ended: true });

    let (fake, path) = FakeHerdr::start();
    fake.on_error("pane.read", "internal", "boom");
    let c = client(&path).await;
    assert!(timeout(T, screen(&c, "w1:p1")).await.unwrap().unwrap_err().contains("boom"));
}

#[tokio::test]
async fn input_sends_text_and_keys() {
    let (fake, path) = FakeHerdr::start();
    fake.on("pane.send_input", json!({ "type": "ok" }));
    let c = client(&path).await;
    timeout(T, input(&c, "w1:p1", Some("héllo ".into()), None)).await.unwrap().unwrap();
    timeout(T, input(&c, "w1:p1", None, Some(vec!["ctrl+c".into(), "enter".into()]))).await.unwrap().unwrap();
    let sent: Vec<Value> =
        fake.requests().into_iter().filter(|r| r["method"] == "pane.send_input").map(|r| r["params"].clone()).collect();
    assert_eq!(
        sent,
        vec![
            json!({ "pane_id": "w1:p1", "text": "héllo " }),
            json!({ "pane_id": "w1:p1", "keys": ["ctrl+c", "enter"] }),
        ]
    );
}

/// Live check against a named dev session (never the default one):
///   HERDR_SESSION=agents-ui-dev herdr server &
///   HERDR_SOCKET_PATH=~/.config/herdr/sessions/agents-ui-dev/herdr.sock HERDR_LIVE_PANE=<pane id> \
///     cargo test --manifest-path src-tauri/Cargo.toml --test terminal_test -- --ignored --nocapture
/// The pane should run a shell. Types an echo command and reads it back.
#[tokio::test]
#[ignore]
async fn real_herdr_send_input_and_read_screen() {
    use agents_lib::herdr::client::default_socket_path;
    assert!(std::env::var_os("HERDR_SOCKET_PATH").is_some(), "set HERDR_SOCKET_PATH to a named dev session socket");
    let pane = std::env::var("HERDR_LIVE_PANE").expect("HERDR_LIVE_PANE");
    let c = client(&default_socket_path()).await;
    input(&c, &pane, Some("echo term-$((20+22))".into()), None).await.unwrap();
    input(&c, &pane, None, Some(vec!["enter".into()])).await.unwrap();
    let mut last = String::new();
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        last = screen(&c, &pane).await.unwrap().text;
        if last.contains("term-42") {
            println!("{last:?}");
            return;
        }
    }
    panic!("never saw term-42: {last:?}");
}
