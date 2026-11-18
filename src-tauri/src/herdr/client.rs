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
use std::time::Duration;

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

/// How long to wait for herdr to accept a connection or answer a request
/// (including a subscription ack) before treating it as `Disconnected`.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

async fn open(path: &Path, limit: Duration) -> Result<UnixStream, HerdrError> {
    match tokio::time::timeout(limit, UnixStream::connect(path)).await {
        Err(_) => Err(HerdrError::Disconnected),
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(e)) => Err(match e.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => HerdrError::NotRunning,
            _ => HerdrError::Protocol(format!("connect {}: {e}", path.display())),
        }),
    }
}

type Conn = (Lines<BufReader<OwnedReadHalf>>, OwnedWriteHalf);

/// Send `req` on a new connection and wait (up to `limit` overall) for the
/// response with its id. Returns the connection so a subscription can keep
/// streaming from it.
async fn request(path: &Path, req: &Request, limit: Duration) -> Result<(Value, Conn), HerdrError> {
    let exchange = async {
        let (read, mut write) = open(path, limit).await?.into_split();
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
                other => log::debug!("herdr {}: skipping unrelated line {other:?}", req.method),
            }
        }
    };
    tokio::time::timeout(limit, exchange).await.unwrap_or(Err(HerdrError::Disconnected))
}

/// Handle for request/response calls. Cheap to clone; concurrent calls are
/// independent (each has its own connection) and responses are matched by id.
#[derive(Clone)]
pub struct HerdrClient {
    path: Arc<PathBuf>,
    next_id: Arc<AtomicU64>,
    timeout: Duration,
}

impl fmt::Debug for HerdrClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HerdrClient").field("path", &self.path).finish()
    }
}

impl HerdrClient {
    /// Check that herdr is listening at `path` and return a client for it.
    pub async fn connect(path: &Path) -> Result<Self, HerdrError> {
        Self::connect_with_timeout(path, DEFAULT_TIMEOUT).await
    }

    /// Like `connect`, with `timeout` applied to the probe and to every call.
    pub async fn connect_with_timeout(path: &Path, timeout: Duration) -> Result<Self, HerdrError> {
        drop(open(path, timeout).await?);
        Ok(Self { path: Arc::new(path.to_path_buf()), next_id: Arc::new(AtomicU64::new(1)), timeout })
    }

    /// A client for `path` without probing it (tests that only store one).
    #[cfg(test)]
    pub(crate) fn unconnected(path: &Path) -> Self {
        Self { path: Arc::new(path.to_path_buf()), next_id: Arc::new(AtomicU64::new(1)), timeout: DEFAULT_TIMEOUT }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let req = Request { id, method: method.into(), params };
        request(&self.path, &req, self.timeout).await.map(|(result, _)| result)
    }
}

/// Open a dedicated connection, send `events.subscribe` with `subscriptions`
/// (a JSON array), wait for the ack and stream pushed events. The receiver
/// closes when the socket closes or a read fails; dropping it closes the socket.
pub async fn subscribe(path: &Path, subscriptions: Value) -> Result<mpsc::Receiver<HerdrEvent>, HerdrError> {
    subscribe_with_timeout(path, subscriptions, DEFAULT_TIMEOUT).await
}

/// Like `subscribe`, with `timeout` applied to connecting and the ack.
pub async fn subscribe_with_timeout(
    path: &Path,
    subscriptions: Value,
    timeout: Duration,
) -> Result<mpsc::Receiver<HerdrEvent>, HerdrError> {
    let req = Request {
        id: "subscribe".into(),
        method: "events.subscribe".into(),
        params: json!({ "subscriptions": subscriptions }),
    };
    let (_ack, (mut lines, write)) = request(path, &req, timeout).await?;

    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let _write = write; // keep our half open for the life of the subscription
        loop {
            tokio::select! {
                _ = tx.closed() => break,
                line = lines.next_line() => {
                    let line = match line {
                        Ok(Some(line)) => line,
                        Ok(None) => break,
                        Err(e) => {
                            log::warn!("herdr event stream read failed, closing: {e}");
                            break;
                        }
                    };
                    match parse_line(&line) {
                        Ok(Incoming::Event(ev)) => {
                            if tx.send(ev).await.is_err() {
                                break;
                            }
                        }
                        Ok(other) => log::debug!("herdr event stream: ignoring {other:?}"),
                        Err(e) => log::warn!("herdr event stream: unparseable line ({e}): {}", line.chars().take(200).collect::<String>()),
                    }
                }
            }
        }
    });
    Ok(rx)
}
