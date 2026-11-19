//! Tauri commands exposed to the UI.

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Runtime, State};
use tokio::sync::Mutex;

use crate::agents::prompt::{APPROVE_KEYS, Conversation, DENY_KEYS, clean_output, detect_permission, reply_params};
use crate::herdr::client::{HerdrClient, HerdrError};
use crate::herdr::model::AgentStatus;
use crate::publish;
use crate::state::{AppState, UiState};

#[tauri::command]
pub async fn get_state(state: State<'_, Mutex<AppState>>) -> Result<UiState, String> {
    Ok(state.lock().await.to_ui())
}

/// Mark an agent's current state as seen and push the updated state.
#[tauri::command]
pub async fn mark_seen<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Mutex<AppState>>,
    pane_id: String,
) -> Result<(), String> {
    let mut s = state.lock().await;
    let seq = s.model.agents.get(&pane_id).map(|a| a.state_change_seq);
    if let Some(seq) = seq {
        s.attention.mark_seen(&pane_id, seq);
    }
    let ui = s.to_ui();
    drop(s);
    publish(&app, &ui);
    Ok(())
}

/// Ask herdr to move a Space. The UI updates when herdr's event arrives.
#[tauri::command]
pub async fn move_space(
    state: State<'_, Mutex<AppState>>,
    workspace_id: String,
    insert_index: usize,
) -> Result<(), String> {
    let client = client(&state).await?;
    client
        .call("workspace.move", json!({ "workspace_id": workspace_id, "insert_index": insert_index }))
        .await
        .map(|_| ())
        .map_err(|e| format!("couldn't move the space: {e}"))
}

/// Read an agent's recent output and any pending permission prompt.
#[tauri::command]
pub async fn read_conversation(state: State<'_, Mutex<AppState>>, pane_id: String) -> Result<Conversation, String> {
    let (client, status) = client_and_status(&state, &pane_id).await?;
    conversation(&client, &pane_id, status).await
}

/// Send `text` to the agent as a prompt, exactly as typed.
#[tauri::command]
pub async fn send_reply(state: State<'_, Mutex<AppState>>, pane_id: String, text: String) -> Result<(), String> {
    let client = client(&state).await?;
    reply(&client, &pane_id, &text).await
}

/// Answer the permission prompt showing `question` with "yes".
#[tauri::command]
pub async fn approve(state: State<'_, Mutex<AppState>>, pane_id: String, question: String) -> Result<(), String> {
    let (client, status) = client_and_status(&state, &pane_id).await?;
    answer(&client, &pane_id, status, &question, APPROVE_KEYS).await
}

/// Answer the permission prompt showing `question` with "no".
#[tauri::command]
pub async fn deny(state: State<'_, Mutex<AppState>>, pane_id: String, question: String) -> Result<(), String> {
    let (client, status) = client_and_status(&state, &pane_id).await?;
    answer(&client, &pane_id, status, &question, DENY_KEYS).await
}

/// The pane's visible screen, ANSI-styled, for the Terminal tab.
#[tauri::command]
pub async fn read_screen(state: State<'_, Mutex<AppState>>, pane_id: String) -> Result<Screen, String> {
    let client = client(&state).await?;
    screen(&client, &pane_id).await
}

/// Forward pasted `text` (herdr marks it as a paste if the app wants that) and/or
/// herdr key-combo strings `keys` to the pane.
#[tauri::command]
pub async fn send_input(
    state: State<'_, Mutex<AppState>>,
    pane_id: String,
    text: Option<String>,
    keys: Option<Vec<String>>,
) -> Result<(), String> {
    let client = client(&state).await?;
    input(&client, &pane_id, text, keys).await
}

/// Write `text` to the pane as typed, byte-for-byte (keystrokes, mouse reports).
#[tauri::command]
pub async fn send_text(state: State<'_, Mutex<AppState>>, pane_id: String, text: String) -> Result<(), String> {
    let client = client(&state).await?;
    type_text(&client, &pane_id, &text).await
}

/// Scroll `pane_id` to `offset_from_bottom` lines up (herdr clamps it).
#[tauri::command]
pub async fn scroll_screen(
    state: State<'_, Mutex<AppState>>,
    pane_id: String,
    offset_from_bottom: u64,
) -> Result<Scroll, String> {
    let client = client(&state).await?;
    scroll(&client, &pane_id, offset_from_bottom).await
}

/// The pane's visible screen. `ended` when herdr no longer knows the pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Screen {
    pub text: String,
    pub ended: bool,
    pub scroll: Scroll,
}

/// How far the pane is scrolled up from the bottom, in lines, and how far it can go.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Scroll {
    pub offset: u64,
    pub max: u64,
}

/// Read `pane_id`'s visible screen with its ANSI styling (`format: ansi`,
/// `strip_ansi: false`) so the terminal view can render colours.
pub async fn screen(client: &HerdrClient, pane_id: &str) -> Result<Screen, String> {
    let params = json!({ "pane_id": pane_id, "source": "visible", "format": "ansi", "strip_ansi": false });
    let (text, info) = tokio::join!(read_text(client, params), client.call("pane.get", json!({ "pane_id": pane_id })));
    let ended = Screen { text: String::new(), ended: true, scroll: Scroll::default() };
    match (text, info.and_then(|r| scroll_of(&r))) {
        (Ok(text), Ok(scroll)) => Ok(Screen { text, ended: false, scroll }),
        (Err(HerdrError::Api { code, .. }), _) | (_, Err(HerdrError::Api { code, .. })) if is_not_found(&code) => {
            Ok(ended)
        }
        (Err(e), _) | (_, Err(e)) => Err(format!("couldn't read the terminal: {e}")),
    }
}

/// Write `text` via `pane.send_text`, which, unlike `pane.send_input`, never
/// wraps it in bracketed-paste markers. Claude Code treats a paste as input
/// and jumps to the bottom, so mouse reports and typing must not look like one.
pub async fn type_text(client: &HerdrClient, pane_id: &str, text: &str) -> Result<(), String> {
    client
        .call("pane.send_text", json!({ "pane_id": pane_id, "text": text }))
        .await
        .map(|_| ())
        .map_err(|e| format!("couldn't send input: {e}"))
}

/// Scroll the pane via `pane.scroll` and return where it landed.
pub async fn scroll(client: &HerdrClient, pane_id: &str, offset_from_bottom: u64) -> Result<Scroll, String> {
    client
        .call("pane.scroll", json!({ "pane_id": pane_id, "offset_from_bottom": offset_from_bottom }))
        .await
        .and_then(|r| scroll_of(&r))
        .map_err(|e| format!("couldn't scroll the terminal: {e}"))
}

/// The `scroll` of a `pane_info` result.
fn scroll_of(result: &Value) -> Result<Scroll, HerdrError> {
    let s = &result["pane"]["scroll"];
    match (s["offset_from_bottom"].as_u64(), s["max_offset_from_bottom"].as_u64()) {
        (Some(offset), Some(max)) => Ok(Scroll { offset, max }),
        _ => Err(HerdrError::Protocol("pane result has no pane.scroll".into())),
    }
}

/// Send text and/or key combos to the pane via `pane.send_input`.
pub async fn input(
    client: &HerdrClient,
    pane_id: &str,
    text: Option<String>,
    keys: Option<Vec<String>>,
) -> Result<(), String> {
    let mut params = json!({ "pane_id": pane_id });
    if let Some(text) = text {
        params["text"] = json!(text);
    }
    if let Some(keys) = keys {
        params["keys"] = json!(keys);
    }
    client.call("pane.send_input", params).await.map(|_| ()).map_err(|e| format!("couldn't send input: {e}"))
}

/// How much recent output the conversation view shows.
const CONVERSATION_LINES: u32 = 200;

/// Read `pane_id`'s recent output, cleaned, plus its permission prompt if
/// it is `blocked`. `status` is the agent's status in the model, `None` when
/// it is no longer there; that, or herdr not knowing the pane, means the
/// session ended.
pub async fn conversation(
    client: &HerdrClient,
    pane_id: &str,
    status: Option<AgentStatus>,
) -> Result<Conversation, String> {
    let ended = Conversation { text: String::new(), permission: None, ended: true };
    let raw = match read_text(
        client,
        json!({ "pane_id": pane_id, "source": "recent_unwrapped", "lines": CONVERSATION_LINES }),
    )
    .await
    {
        Ok(text) => text,
        Err(HerdrError::Api { code, .. }) if is_not_found(&code) => return Ok(ended),
        Err(e) => return Err(format!("couldn't read the conversation: {e}")),
    };
    let text = clean_output(&raw);
    let Some(status) = status else {
        return Ok(Conversation { text, ..ended });
    };
    let permission = if status == AgentStatus::Blocked {
        match read_text(client, json!({ "pane_id": pane_id, "source": "detection" })).await {
            Ok(screen) => detect_permission(&screen),
            Err(HerdrError::Api { code, .. }) if is_not_found(&code) => {
                return Ok(Conversation { text, ..ended });
            }
            Err(e) => return Err(format!("couldn't read the conversation: {e}")),
        }
    } else {
        None
    };
    Ok(Conversation { text, permission, ended: false })
}

/// herdr reports a missing pane/agent as `pane_not_found`, `agent_not_found`, ...
fn is_not_found(code: &str) -> bool {
    code.ends_with("not_found")
}

async fn read_text(client: &HerdrClient, params: Value) -> Result<String, HerdrError> {
    let result = client.call("pane.read", params).await?;
    result["read"]["text"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| HerdrError::Protocol("pane.read result has no read.text".into()))
}

/// Send `text` to the agent via `agent.prompt`, byte-for-byte.
pub async fn reply(client: &HerdrClient, pane_id: &str, text: &str) -> Result<(), String> {
    let params = reply_params(pane_id, text)?;
    client.call("agent.prompt", params).await.map(|_| ()).map_err(|e| format!("couldn't send the reply: {e}"))
}

/// Why an Approve/Deny was refused: what the user read is no longer the
/// prompt on screen.
pub const PROMPT_CHANGED: &str = "The prompt changed — check it again.";

/// Press `keys` to answer the permission prompt, but only if the agent is
/// `blocked` and the prompt on screen right now is the one the user read
/// (`question`); otherwise nothing is sent and `PROMPT_CHANGED` is returned.
pub async fn answer(
    client: &HerdrClient,
    pane_id: &str,
    status: Option<AgentStatus>,
    question: &str,
    keys: &[&str],
) -> Result<(), String> {
    if status != Some(AgentStatus::Blocked) {
        return Err(PROMPT_CHANGED.into());
    }
    let screen = read_text(client, json!({ "pane_id": pane_id, "source": "detection" }))
        .await
        .map_err(|e| format!("couldn't read the prompt: {e}"))?;
    if detect_permission(&screen).is_none_or(|p| p.question != question) {
        return Err(PROMPT_CHANGED.into());
    }
    press(client, pane_id, keys).await
}

/// Send key presses to the pane via `pane.send_keys`.
pub async fn press(client: &HerdrClient, pane_id: &str, keys: &[&str]) -> Result<(), String> {
    client
        .call("pane.send_keys", json!({ "pane_id": pane_id, "keys": keys }))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

async fn client_and_status(
    state: &Mutex<AppState>,
    pane_id: &str,
) -> Result<(HerdrClient, Option<AgentStatus>), String> {
    let s = state.lock().await;
    let client = s.client.clone().ok_or_else(|| "herdr isn't running".to_string())?;
    Ok((client, s.model.agents.get(pane_id).map(|a| a.status)))
}

async fn client(state: &Mutex<AppState>) -> Result<HerdrClient, String> {
    state.lock().await.client.clone().ok_or_else(|| "herdr isn't running".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn client_errors_when_herdr_is_not_connected() {
        let state = Mutex::new(AppState::default());
        assert_eq!(client(&state).await.unwrap_err(), "herdr isn't running");
    }
}
