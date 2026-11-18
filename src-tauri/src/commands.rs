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
    let (client, status) = {
        let s = state.lock().await;
        let client = s.client.clone().ok_or_else(|| "herdr isn't running".to_string())?;
        (client, s.model.agents.get(&pane_id).map(|a| a.status))
    };
    conversation(&client, &pane_id, status).await
}

/// Send `text` to the agent as a prompt, exactly as typed.
#[tauri::command]
pub async fn send_reply(state: State<'_, Mutex<AppState>>, pane_id: String, text: String) -> Result<(), String> {
    let client = client(&state).await?;
    reply(&client, &pane_id, &text).await
}

/// Answer the agent's permission prompt with "yes".
#[tauri::command]
pub async fn approve(state: State<'_, Mutex<AppState>>, pane_id: String) -> Result<(), String> {
    let client = client(&state).await?;
    press(&client, &pane_id, APPROVE_KEYS).await.map_err(|e| format!("couldn't approve: {e}"))
}

/// Answer the agent's permission prompt with "no".
#[tauri::command]
pub async fn deny(state: State<'_, Mutex<AppState>>, pane_id: String) -> Result<(), String> {
    let client = client(&state).await?;
    press(&client, &pane_id, DENY_KEYS).await.map_err(|e| format!("couldn't deny: {e}"))
}

/// The pane's visible screen, ANSI-styled, for the Terminal tab.
#[tauri::command]
pub async fn read_screen(state: State<'_, Mutex<AppState>>, pane_id: String) -> Result<Screen, String> {
    let client = client(&state).await?;
    screen(&client, &pane_id).await
}

/// Forward typed `text` and/or herdr key-combo strings `keys` to the pane.
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

/// The pane's visible screen. `ended` when herdr no longer knows the pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Screen {
    pub text: String,
    pub ended: bool,
}

/// Read `pane_id`'s visible screen with its ANSI styling (`format: ansi`,
/// `strip_ansi: false`) so the terminal view can render colours.
pub async fn screen(client: &HerdrClient, pane_id: &str) -> Result<Screen, String> {
    let params = json!({ "pane_id": pane_id, "source": "visible", "format": "ansi", "strip_ansi": false });
    match read_text(client, params).await {
        Ok(text) => Ok(Screen { text, ended: false }),
        Err(HerdrError::Api { code, .. }) if is_not_found(&code) => Ok(Screen { text: String::new(), ended: true }),
        Err(e) => Err(format!("couldn't read the terminal: {e}")),
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

/// Send key presses to the pane via `pane.send_keys`.
pub async fn press(client: &HerdrClient, pane_id: &str, keys: &[&str]) -> Result<(), String> {
    client
        .call("pane.send_keys", json!({ "pane_id": pane_id, "keys": keys }))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
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
