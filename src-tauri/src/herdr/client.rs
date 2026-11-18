//! Async client for the herdr Unix socket. Plain tokio; no tauri types.
//!
//! herdr (0.9.5) serves ONE request per connection and then closes it, so
//! `HerdrClient::call` opens a fresh connection per call. `subscribe` holds a
//! dedicated long-lived connection for pushed events.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;

use super::protocol::{HerdrEvent, Incoming, Request, parse_line};

/// `HERDR_SOCKET_PATH` if set, else `~/.config/herdr/herdr.sock`.
pub fn default_socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("HERDR_SOCKET_PATH") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".config/herdr/herdr.sock")
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HerdrError {
    #[error("herdr is not running")]
    NotRunning,
    #[error("disconnected from herdr")]
    Disconnected,
    #[error("herdr error {code}: {message}")]
    Api { code: String, message: String },
    #[error("protocol error: {0}")]
    Protocol(String),
}

async fn open(path: &Path) -> Result<UnixStream, HerdrError> {
    UnixStream::connect(path).await.map_err(|e| match e.kind() {
        ErrorKind::NotFound | ErrorKind::ConnectionRefused => HerdrError::NotRunning,
        _ => HerdrError::Protocol(format!("connect {}: {e}", path.display())),
    })
}

/// Send `req` on a new connection and wait for the response with its id.
/// Returns the reader so a subscription can keep streaming from it.
type Conn = (Lines<BufReader<OwnedReadHalf>>, OwnedWriteHalf);

async fn request(path: &Path, req: &Request) -> Result<(Value, Conn), HerdrError> {
    let (read, mut write) = open(path).await?.into_split();
    write.write_all(req.to_line().as_bytes()).await.map_err(|_| HerdrError::Disconnected)?;
    let mut lines = BufReader::new(read).lines();
    loop {
        let line = lines
            .next_line()
            .await
            .map_err(|_| HerdrError::Disconnected)?
            .ok_or(HerdrError::Disconnected)?;
        match parse_line(&line).map_err(|e| HerdrError::Protocol(e.to_string()))? {
            Incoming::Response { id, result } if id == req.id => return Ok((result, (lines, write))),
            Incoming::Error { id, code, message } if id == req.id => {
                return Err(HerdrError::Api { code, message });
            }
            _ => continue,
        }
    }
}

/// Handle for request/response calls. Cheap to clone; concurrent calls are
/// independent (each has its own connection) and responses are matched by id.
#[derive(Clone)]
pub struct HerdrClient {
    path: Arc<PathBuf>,
    next_id: Arc<AtomicU64>,
}

impl fmt::Debug for HerdrClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HerdrClient").field("path", &self.path).finish()
    }
}

impl HerdrClient {
    /// Check that herdr is listening at `path` and return a client for it.
    pub async fn connect(path: &Path) -> Result<Self, HerdrError> {
        drop(open(path).await?);
        Ok(Self { path: Arc::new(path.to_path_buf()), next_id: Arc::new(AtomicU64::new(1)) })
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let req = Request { id, method: method.into(), params };
        request(&self.path, &req).await.map(|(result, _)| result)
    }
}

/// Open a dedicated connection, send `events.subscribe` with `subscriptions`
/// (a JSON array), wait for the ack and stream pushed events. The receiver
/// closes when the socket closes; dropping it closes the socket.
pub async fn subscribe(path: &Path, subscriptions: Value) -> Result<mpsc::Receiver<HerdrEvent>, HerdrError> {
    let req = Request {
        id: "subscribe".into(),
        method: "events.subscribe".into(),
        params: json!({ "subscriptions": subscriptions }),
    };
    let (_ack, (mut lines, write)) = request(path, &req).await?;

    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let _write = write; // keep our half open for the life of the subscription
        loop {
            tokio::select! {
                _ = tx.closed() => break,
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { break };
                    if let Ok(Incoming::Event(ev)) = parse_line(&line)
                        && tx.send(ev).await.is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    Ok(rx)
}
