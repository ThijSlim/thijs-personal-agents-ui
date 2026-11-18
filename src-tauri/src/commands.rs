//! Tauri commands exposed to the UI.

use serde_json::json;
use tauri::{AppHandle, Runtime, State};
use tokio::sync::Mutex;

use crate::herdr::client::HerdrClient;
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
