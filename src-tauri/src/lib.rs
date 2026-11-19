pub mod agents;
pub mod commands;
pub mod herdr;
pub mod state;

use tauri::{AppHandle, Emitter, Manager, Runtime, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{Mutex, mpsc};

use herdr::client::default_socket_path;
use herdr::supervisor::{self, SessionUpdate};
use state::{AppState, Notice, UiState, badge_count};

const MAIN_WINDOW: &str = "main";
/// Key in a notification's `extra` payload holding the agent's pane id.
const PANE_ID_KEY: &str = "paneId";

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        if let Err(e) = w.unminimize() {
            log::warn!("unminimize main window failed: {e}");
        }
        if let Err(e) = w.show() {
            log::warn!("show main window failed: {e}");
        }
        if let Err(e) = w.set_focus() {
            log::warn!("focus main window failed: {e}");
        }
    }
}

/// Push a UI state to the front end and the Dock badge.
pub fn publish<R: Runtime>(app: &AppHandle<R>, ui: &UiState) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW)
        && let Err(e) = w.set_badge_count(badge_count(ui))
    {
        log::warn!("set_badge_count failed: {e}");
    }
    if let Err(e) = app.emit("state-changed", ui) {
        log::warn!("emit state-changed failed: {e}");
    }
}

fn notify<R: Runtime>(app: &AppHandle<R>, n: &Notice) {
    log::debug!("posting notification for {}: {:?} / {:?}", n.pane_id, n.title, n.body);
    let res = app
        .notification()
        .builder()
        .title(&n.title)
        .body(&n.body)
        .extra(PANE_ID_KEY, &n.pane_id)
        .show();
    if let Err(e) = res {
        log::warn!("notification failed: {e}");
    }
}

/// Apply supervisor updates to `AppState`, post notifications, then publish.
async fn consume<R: Runtime>(app: AppHandle<R>, mut rx: mpsc::Receiver<SessionUpdate>) {
    while let Some(update) = rx.recv().await {
        let state = app.state::<Mutex<AppState>>();
        let mut s = state.lock().await;
        let notices = s.apply(update);
        let ui = s.to_ui();
        drop(s);
        log::debug!(
            "state: {:?}, {} spaces, {} needs-you, {} notices",
            ui.connection,
            ui.spaces.len(),
            ui.needs_you.len(),
            notices.len()
        );
        for n in &notices {
            notify(&app, n);
        }
        publish(&app, &ui);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let level = if cfg!(debug_assertions) { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(level)
                .clear_targets()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: None }),
                ])
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .manage(Mutex::new(AppState::default()))
        .setup(|app| {
            let handle = app.handle().clone();
            app.notification().on_action(move |performed| {
                if performed.action_id() != "tap" {
                    return;
                }
                show_main(&handle);
                let pane_id = performed
                    .notification()
                    .and_then(|n| n.extra().get(PANE_ID_KEY))
                    .and_then(|v| v.as_str());
                match pane_id {
                    Some(pane_id) => {
                        if let Err(e) = handle.emit("select-agent", pane_id) {
                            log::warn!("emit select-agent failed: {e}");
                        }
                    }
                    None => log::debug!("notification click without {PANE_ID_KEY} in extra"),
                }
            })?;
            let (tx, rx) = mpsc::channel(64);
            tauri::async_runtime::spawn(supervisor::run(default_socket_path(), tx));
            tauri::async_runtime::spawn(consume(app.handle().clone(), rx));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == MAIN_WINDOW
            {
                api.prevent_close();
                if let Err(e) = window.hide() {
                    log::warn!("hide main window failed: {e}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::mark_seen,
            commands::move_space,
            commands::read_conversation,
            commands::send_reply,
            commands::approve,
            commands::deny,
            commands::read_screen,
            commands::send_input,
            commands::send_text,
            commands::scroll_screen,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
