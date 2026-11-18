mod fake_herdr;

use std::time::Duration;

use agents_lib::agents::prompt::{APPROVE_KEYS, DENY_KEYS, detect_permission};
use agents_lib::commands::{PROMPT_CHANGED, answer, conversation, press, reply};
use agents_lib::herdr::client::HerdrClient;
use agents_lib::herdr::model::AgentStatus;
use fake_herdr::FakeHerdr;
use serde_json::{Value, json};
use tokio::time::timeout;

const T: Duration = Duration::from_secs(2);
const PERMISSION: &str = include_str!("fixtures/claude_permission.txt");

fn read_result(text: &str) -> Value {
    json!({
        "type": "pane_read",
        "read": {
            "pane_id": "w1:p1", "workspace_id": "w1", "tab_id": "w1:t1",
            "source": "recent_unwrapped", "format": "text",
            "text": text, "revision": 3, "truncated": false
        }
    })
}

async fn client(path: &std::path::Path) -> HerdrClient {
    timeout(T, HerdrClient::connect(path)).await.unwrap().unwrap()
}

fn reads(fake: &FakeHerdr) -> Vec<Value> {
    fake.requests().into_iter().filter(|r| r["method"] == "pane.read").map(|r| r["params"].clone()).collect()
}

#[tokio::test]
async fn reads_recent_unwrapped_and_detects_permission_only_when_blocked() {
    let (fake, path) = FakeHerdr::start();
    fake.on("pane.read", read_result(&format!("\x1b[1mhello\x1b[0m\n{PERMISSION}\n\n\n")));
    let c = client(&path).await;

    let conv = timeout(T, conversation(&c, "w1:p1", Some(AgentStatus::Blocked))).await.unwrap().unwrap();
    assert!(conv.text.starts_with("hello\n"), "{:?}", conv.text);
    assert!(!conv.text.ends_with('\n'));
    assert!(!conv.ended);
    assert!(conv.permission.expect("permission").question.contains("touch probe.txt"));
    assert_eq!(
        reads(&fake),
        vec![
            json!({ "pane_id": "w1:p1", "source": "recent_unwrapped", "lines": 200 }),
            json!({ "pane_id": "w1:p1", "source": "detection" }),
        ]
    );

    let conv = timeout(T, conversation(&c, "w1:p1", Some(AgentStatus::Working))).await.unwrap().unwrap();
    assert_eq!(conv.permission, None);
    assert_eq!(reads(&fake).len(), 3, "no detection read unless blocked");
}

#[tokio::test]
async fn pane_not_found_means_ended() {
    let (fake, path) = FakeHerdr::start();
    fake.on_error("pane.read", "pane_not_found", "pane w1:p1 not found");
    let c = client(&path).await;
    let conv = timeout(T, conversation(&c, "w1:p1", Some(AgentStatus::Blocked))).await.unwrap().unwrap();
    assert!(conv.ended);
    assert_eq!(conv.text, "");
    assert_eq!(conv.permission, None);
}

#[tokio::test]
async fn absent_from_model_means_ended_but_keeps_text() {
    let (fake, path) = FakeHerdr::start();
    fake.on("pane.read", read_result("last words\n"));
    let c = client(&path).await;
    let conv = timeout(T, conversation(&c, "w1:p1", None)).await.unwrap().unwrap();
    assert!(conv.ended);
    assert_eq!(conv.text, "last words");
    assert_eq!(conv.permission, None);
}

#[tokio::test]
async fn other_read_errors_are_reported() {
    let (fake, path) = FakeHerdr::start();
    fake.on_error("pane.read", "internal", "boom");
    let c = client(&path).await;
    let err = timeout(T, conversation(&c, "w1:p1", Some(AgentStatus::Idle))).await.unwrap().unwrap_err();
    assert!(err.contains("boom"), "{err}");
}

#[tokio::test]
async fn reply_sends_agent_prompt_with_exact_text() {
    let (fake, path) = FakeHerdr::start();
    fake.on("agent.prompt", json!({ "type": "agent_prompted", "agent": {} }));
    let c = client(&path).await;
    let text = "line1\nline2 — ✓ 日本 ";
    timeout(T, reply(&c, "w1:p1", text)).await.unwrap().unwrap();
    let req = fake.wait_for_request(|r| r["method"] == "agent.prompt").await;
    assert_eq!(req["params"], json!({ "target": "w1:p1", "text": text }));

    let err = timeout(T, reply(&c, "w1:p1", " \n ")).await.unwrap().unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(fake.requests().iter().filter(|r| r["method"] == "agent.prompt").count(), 1);
}

#[tokio::test]
async fn press_sends_keys() {
    let (fake, path) = FakeHerdr::start();
    fake.on("pane.send_keys", json!({ "type": "ok" }));
    let c = client(&path).await;
    timeout(T, press(&c, "w1:p1", APPROVE_KEYS)).await.unwrap().unwrap();
    timeout(T, press(&c, "w1:p1", DENY_KEYS)).await.unwrap().unwrap();
    let keys: Vec<Value> =
        fake.requests().into_iter().filter(|r| r["method"] == "pane.send_keys").map(|r| r["params"].clone()).collect();
    assert_eq!(
        keys,
        vec![json!({ "pane_id": "w1:p1", "keys": ["1"] }), json!({ "pane_id": "w1:p1", "keys": ["esc"] })]
    );
}

fn sent_keys(fake: &FakeHerdr) -> Vec<Value> {
    fake.requests().into_iter().filter(|r| r["method"] == "pane.send_keys").map(|r| r["params"].clone()).collect()
}

#[tokio::test]
async fn answer_presses_keys_when_the_prompt_is_the_one_shown() {
    let (fake, path) = FakeHerdr::start();
    fake.on("pane.read", read_result(PERMISSION));
    fake.on("pane.send_keys", json!({ "type": "ok" }));
    let c = client(&path).await;
    let question = detect_permission(PERMISSION).unwrap().question;
    timeout(T, answer(&c, "w1:p1", Some(AgentStatus::Blocked), &question, APPROVE_KEYS)).await.unwrap().unwrap();
    assert_eq!(reads(&fake), vec![json!({ "pane_id": "w1:p1", "source": "detection" })]);
    assert_eq!(sent_keys(&fake), vec![json!({ "pane_id": "w1:p1", "keys": ["1"] })]);
}

#[tokio::test]
async fn answer_refuses_when_the_prompt_changed() {
    let (fake, path) = FakeHerdr::start();
    fake.on("pane.read", read_result(&PERMISSION.replace("touch probe.txt", "rm -rf build")));
    fake.on("pane.send_keys", json!({ "type": "ok" }));
    let c = client(&path).await;
    let question = detect_permission(PERMISSION).unwrap().question;
    let err = timeout(T, answer(&c, "w1:p1", Some(AgentStatus::Blocked), &question, DENY_KEYS)).await.unwrap().unwrap_err();
    assert_eq!(err, PROMPT_CHANGED);
    assert!(sent_keys(&fake).is_empty());

    // No prompt on screen at all.
    fake.on("pane.read", read_result("all done\n"));
    let err = timeout(T, answer(&c, "w1:p1", Some(AgentStatus::Blocked), &question, APPROVE_KEYS)).await.unwrap().unwrap_err();
    assert_eq!(err, PROMPT_CHANGED);
    assert!(sent_keys(&fake).is_empty());
}

#[tokio::test]
async fn answer_refuses_when_not_blocked() {
    let (fake, path) = FakeHerdr::start();
    fake.on("pane.read", read_result(PERMISSION));
    fake.on("pane.send_keys", json!({ "type": "ok" }));
    let c = client(&path).await;
    let question = detect_permission(PERMISSION).unwrap().question;
    for status in [Some(AgentStatus::Working), Some(AgentStatus::Idle), None] {
        let err = timeout(T, answer(&c, "w1:p1", status, &question, APPROVE_KEYS)).await.unwrap().unwrap_err();
        assert_eq!(err, PROMPT_CHANGED, "{status:?}");
    }
    assert!(sent_keys(&fake).is_empty());
    assert!(reads(&fake).is_empty(), "no read unless blocked");
}

#[tokio::test]
async fn answer_reports_read_errors_without_pressing() {
    let (fake, path) = FakeHerdr::start();
    fake.on_error("pane.read", "internal", "boom");
    fake.on("pane.send_keys", json!({ "type": "ok" }));
    let c = client(&path).await;
    let err = timeout(T, answer(&c, "w1:p1", Some(AgentStatus::Blocked), "q?", APPROVE_KEYS)).await.unwrap().unwrap_err();
    assert!(err.contains("boom"), "{err}");
    assert!(sent_keys(&fake).is_empty());
}

/// Live check against a named dev session with Claude Code in `HERDR_LIVE_PANE`
/// (manual permission mode) waiting at a Bash permission prompt for
/// `touch probe.txt`. Checks a reply can't answer the prompt and a stale
/// question presses nothing, approves it, then replies with multi-line
/// unicode text.
///   HERDR_SESSION=agents-ui-dev herdr server &
///   HERDR_SOCKET_PATH=~/.config/herdr/sessions/agents-ui-dev/herdr.sock HERDR_LIVE_PANE=w1:p1 \
///     cargo test --manifest-path src-tauri/Cargo.toml --test conversation_test -- --ignored --nocapture
///   HERDR_SESSION=agents-ui-dev herdr server stop
#[tokio::test]
#[ignore]
async fn real_herdr_approve_and_reply() {
    use agents_lib::herdr::client::default_socket_path;
    assert!(std::env::var_os("HERDR_SOCKET_PATH").is_some(), "set HERDR_SOCKET_PATH to a named dev session socket");
    let pane = std::env::var("HERDR_LIVE_PANE").expect("HERDR_LIVE_PANE");
    let c = client(&default_socket_path()).await;
    let status = |c: HerdrClient, pane: String| async move {
        let v = c.call("agent.get", json!({ "target": pane })).await.unwrap();
        serde_json::from_value::<AgentStatus>(v["agent"]["agent_status"].clone()).unwrap()
    };
    let wait_until = |want: AgentStatus| {
        let (c, pane) = (c.clone(), pane.clone());
        async move {
            for _ in 0..120 {
                if status(c.clone(), pane.clone()).await == want {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            panic!("agent never became {want:?}");
        }
    };

    assert_eq!(status(c.clone(), pane.clone()).await, AgentStatus::Blocked);
    let conv = conversation(&c, &pane, Some(AgentStatus::Blocked)).await.unwrap();
    println!("--- permission question ---\n{}", conv.permission.as_ref().map_or("<none>", |p| &p.question));
    println!(
        "--- conversation tail ---\n{}",
        conv.text.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
    );
    assert!(!conv.ended);
    let question = conv.permission.expect("permission").question;
    assert!(question.contains("touch"));

    // herdr refuses a prompt while the agent is blocked: the reply can't
    // answer the permission prompt, and nothing is typed.
    let err = reply(&c, &pane, "no, use pnpm").await.unwrap_err();
    println!("--- reply while blocked ---\n{err}");
    assert!(err.contains("agent_blocked"), "{err}");
    assert_eq!(status(c.clone(), pane.clone()).await, AgentStatus::Blocked);

    // A question other than the one on screen presses nothing.
    let err = answer(&c, &pane, Some(AgentStatus::Blocked), "Do you want to delete everything?", APPROVE_KEYS)
        .await
        .unwrap_err();
    assert_eq!(err, PROMPT_CHANGED);
    assert_eq!(status(c.clone(), pane.clone()).await, AgentStatus::Blocked);

    answer(&c, &pane, Some(AgentStatus::Blocked), &question, APPROVE_KEYS).await.unwrap();
    wait_until(AgentStatus::Idle).await;
    let conv = conversation(&c, &pane, Some(AgentStatus::Idle)).await.unwrap();
    println!(
        "--- after approve ---\n{}",
        conv.text.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
    );
    assert_eq!(conv.permission, None);

    let text = "Reply with exactly the word OK.\nline2 — ✓ 日本";
    reply(&c, &pane, text).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    wait_until(AgentStatus::Idle).await;
    let conv = conversation(&c, &pane, Some(AgentStatus::Idle)).await.unwrap();
    println!(
        "--- after reply ---\n{}",
        conv.text.lines().rev().take(10).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
    );
    assert!(conv.text.contains("line2 — ✓ 日本"), "reply text not shown in the pane");

    let gone = conversation(&c, "w99:p99", Some(AgentStatus::Idle)).await.unwrap();
    assert!(gone.ended);
}
