//! Test helper: a scriptable fake herdr server on a Unix socket.
//!
//! Accepts any number of concurrent connections, records every request it
//! receives, answers scripted methods, and pushes events to connections that
//! called `events.subscribe`. Like real herdr, each connection carries one
//! request: plain calls are answered and closed. `stop()` closes everything and removes the
//! socket; `restart()` rebinds on the same path.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

#[derive(Clone)]
enum Reply {
    Ok(Value),
    Err { code: String, message: String },
    /// Answer only after the gate is opened.
    Deferred(Value, Arc<Notify>),
    /// Read the request, keep the connection open, never answer.
    Never,
}

/// Releases a deferred reply registered with `FakeHerdr::on_deferred`.
pub struct Gate(Arc<Notify>);

impl Gate {
    pub fn open(&self) {
        self.0.notify_one();
    }
}

#[derive(Default)]
struct State {
    replies: Mutex<HashMap<String, Reply>>,
    requests: Mutex<Vec<Value>>,
    request_seen: Notify,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<String>>>,
    conns: Mutex<Conns>,
}

/// Listener + connection tasks. `running` is checked under the same lock the
/// accept loop uses to register a new connection, so `stop()` cannot miss one.
#[derive(Default)]
struct Conns {
    running: bool,
    tasks: Vec<JoinHandle<()>>,
}

pub struct FakeHerdr {
    state: Arc<State>,
    path: PathBuf,
    dir: PathBuf,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

impl FakeHerdr {
    pub fn start() -> (FakeHerdr, PathBuf) {
        // Keep the path short: macOS limits Unix socket paths to ~104 bytes.
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("fh-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("herdr.sock");
        let fake = FakeHerdr { state: Arc::new(State::default()), path: path.clone(), dir };
        fake.on("ping", json!({"type": "pong"}));
        fake.bind();
        (fake, path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Script a successful result for `method`.
    pub fn on(&self, method: &str, result: Value) {
        self.state.replies.lock().unwrap().insert(method.into(), Reply::Ok(result));
    }

    /// Script an error response for `method`.
    pub fn on_error(&self, method: &str, code: &str, message: &str) {
        self.state
            .replies
            .lock()
            .unwrap()
            .insert(method.into(), Reply::Err { code: code.into(), message: message.into() });
    }

    /// Script a result that is only sent once the returned gate is opened.
    pub fn on_deferred(&self, method: &str, result: Value) -> Gate {
        let gate = Arc::new(Notify::new());
        self.state
            .replies
            .lock()
            .unwrap()
            .insert(method.into(), Reply::Deferred(result, gate.clone()));
        Gate(gate)
    }

    /// Accept requests for `method` but never reply (connection stays open).
    pub fn on_never(&self, method: &str) {
        self.state.replies.lock().unwrap().insert(method.into(), Reply::Never);
    }

    /// Write a raw line (e.g. malformed JSON) to every subscribed connection.
    pub fn push_raw(&self, line: &str) -> usize {
        let line = format!("{line}\n");
        let mut subs = self.state.subscribers.lock().unwrap();
        subs.retain(|tx| tx.send(line.clone()).is_ok());
        subs.len()
    }

    /// Write an event line to every live subscribed connection. Returns how many got it.
    pub fn push_event(&self, event: Value) -> usize {
        let line = format!("{event}\n");
        let mut subs = self.state.subscribers.lock().unwrap();
        subs.retain(|tx| tx.send(line.clone()).is_ok());
        subs.len()
    }

    /// All requests received so far, in arrival order.
    pub fn requests(&self) -> Vec<Value> {
        self.state.requests.lock().unwrap().clone()
    }

    /// Wait (at most 2 s, then panic) until a request matching `pred` has
    /// been received and return it.
    pub async fn wait_for_request(&self, pred: impl Fn(&Value) -> bool) -> Value {
        let wait = async {
            loop {
                let notified = self.state.request_seen.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if let Some(r) = self.requests().into_iter().find(|r| pred(r)) {
                    return r;
                }
                notified.await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), wait)
            .await
            .expect("fake herdr: expected request not received within 2s")
    }

    /// Close the listener and every open connection, and remove the socket file.
    pub fn stop(&self) {
        let mut conns = self.state.conns.lock().unwrap();
        conns.running = false;
        for t in conns.tasks.drain(..) {
            t.abort();
        }
        drop(conns);
        self.state.subscribers.lock().unwrap().clear();
        let _ = std::fs::remove_file(&self.path);
    }

    /// Rebind on the same path after `stop()`.
    pub fn restart(&self) {
        self.stop();
        self.bind();
    }

    fn bind(&self) {
        let _ = std::fs::remove_file(&self.path);
        let listener = UnixListener::bind(&self.path).unwrap();
        let state = self.state.clone();
        let mut conns = self.state.conns.lock().unwrap();
        conns.running = true;
        let accept = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mut conns = state.conns.lock().unwrap();
                if !conns.running {
                    break; // stopped while accepting: drop the stream
                }
                let handle = tokio::spawn(serve(stream, state.clone()));
                conns.tasks.push(handle);
            }
        });
        conns.tasks.push(accept);
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn serve(stream: UnixStream, state: Arc<State>) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    // Like real herdr 0.9.5: a connection carries ONE request. A plain call is
    // answered and the connection closed; `events.subscribe` keeps it open for
    // pushed events.
    let Ok(Some(line)) = lines.next_line().await else { return };
    let Ok(req) = serde_json::from_str::<Value>(&line) else { return };
    let id = req["id"].clone();
    let method = req["method"].as_str().unwrap_or_default().to_string();
    state.requests.lock().unwrap().push(req);
    state.request_seen.notify_waiters();

    let reply = state.replies.lock().unwrap().get(&method).cloned();
    if matches!(reply, Some(Reply::Never)) {
        while let Ok(Some(_)) = lines.next_line().await {}
        return;
    }

    if method == "events.subscribe" {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let _ = tx.send(line_of(json!({"id": id, "result": {"type": "subscription_started"}})));
        state.subscribers.lock().unwrap().push(tx);
        let writer = async {
            while let Some(line) = rx.recv().await {
                if write.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        };
        // Until the peer hangs up (EOF) or a write fails.
        tokio::select! {
            _ = writer => {}
            _ = async { while let Ok(Some(_)) = lines.next_line().await {} } => {}
        }
        return;
    }

    let response = match reply {
        Some(Reply::Ok(result)) => json!({"id": id, "result": result}),
        Some(Reply::Err { code, message }) => json!({"id": id, "error": {"code": code, "message": message}}),
        Some(Reply::Deferred(result, gate)) => {
            gate.notified().await;
            json!({"id": id, "result": result})
        }
        Some(Reply::Never) => unreachable!(),
        None => json!({
            "id": id,
            "error": {"code": "unknown_method", "message": format!("no script for {method}")}
        }),
    };
    let _ = write.write_all(line_of(response).as_bytes()).await;
}

fn line_of(v: Value) -> String {
    format!("{v}\n")
}

/// The captured `session.snapshot` result (`{"snapshot": {...}}`).
pub fn fixture_snapshot() -> Value {
    let v: Value = serde_json::from_str(include_str!("fixtures/snapshot.json")).unwrap();
    v["result"].clone()
}
