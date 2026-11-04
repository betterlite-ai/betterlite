use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::collector::{CollectorAddress, UnavailablePolicy};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::{Fallback, Shared, unavailable};

const RECONNECT_BACKOFF: Duration = Duration::from_secs(1);

enum LocalConnection {
    Unix(tokio::net::UnixStream),
    Tcp(tokio::net::TcpStream),
}

impl LocalConnection {
    async fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(stream) => stream.write(bytes).await,
            Self::Tcp(stream) => stream.write(bytes).await,
        }
    }

    async fn shutdown(&mut self) -> std::io::Result<()> {
        match self {
            Self::Unix(stream) => stream.shutdown().await,
            Self::Tcp(stream) => stream.shutdown().await,
        }
    }

    async fn peer_closed(&self) -> std::io::Result<bool> {
        let mut byte = [0; 1];
        match self {
            Self::Unix(stream) => match stream.try_read(&mut byte) {
                Ok(length) => Ok(length == 0),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
                Err(error) => Err(error),
            },
            Self::Tcp(stream) => stream.peek(&mut byte).await.map(|length| length == 0),
        }
    }
}

pub(super) async fn run_writer(
    address: CollectorAddress,
    mut receiver: mpsc::Receiver<Vec<u8>>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
    shared: Arc<Mutex<Shared>>,
    fallback: Fallback,
    policy: UnavailablePolicy,
    timeout: Duration,
) {
    let mut connection: Option<LocalConnection> = None;
    let mut reconnect_at = tokio::time::Instant::now();
    'writer: loop {
        let line = tokio::select! {
            line = receiver.recv() => match line {
                Some(line) => line,
                None => break,
            },
            _ = &mut shutdown => {
                drain_unsent(&mut receiver, &shared, &fallback, policy).await;
                break;
            }
        };
        if connection.is_none() && tokio::time::Instant::now() >= reconnect_at {
            let connect = tokio::time::timeout(timeout, open(&address));
            tokio::pin!(connect);
            let connected = tokio::select! {
                result = &mut connect => Some(result),
                _ = &mut shutdown => None,
            };
            let Some(connected) = connected else {
                unavailable(&shared, &fallback, policy, line, "shutdown").await;
                drain_unsent(&mut receiver, &shared, &fallback, policy).await;
                break;
            };
            match connected {
                Ok(Ok(stream)) => {
                    connection = Some(stream);
                    if let Ok(mut state) = shared.lock() {
                        state.stats.connected = true;
                    }
                }
                _ => {
                    reconnect_at = tokio::time::Instant::now() + RECONNECT_BACKOFF;
                    if let Ok(mut state) = shared.lock() {
                        state.reconnect_at = Some(reconnect_at);
                    }
                    tracing::warn!("collector connection unavailable; entering one second backoff");
                }
            }
        }
        let Some(stream) = connection.as_mut() else {
            unavailable(&shared, &fallback, policy, line, "sidecar unreachable").await;
            continue;
        };
        if matches!(stream.peer_closed().await, Ok(true) | Err(_)) {
            connection = None;
            reconnect_at = tokio::time::Instant::now() + RECONNECT_BACKOFF;
            if let Ok(mut state) = shared.lock() {
                state.stats.connected = false;
                state.reconnect_at = Some(reconnect_at);
            }
            unavailable(
                &shared,
                &fallback,
                policy,
                line,
                "collector closed the connection",
            )
            .await;
            continue;
        }
        let mut offset = 0;
        let mut write_error = None;
        while offset < line.len() {
            let Some(stream) = connection.as_mut() else {
                break;
            };
            tokio::select! {
                biased;
                result = stream.write(&line[offset..]) => match result {
                    Ok(0) => {
                        write_error = Some(std::io::Error::from(std::io::ErrorKind::WriteZero));
                        break;
                    }
                    Ok(written) => offset += written,
                    Err(error) => {
                        write_error = Some(error);
                        break;
                    }
                },
                _ = &mut shutdown => {
                    connection = None;
                    if offset < line.len() {
                        unavailable(&shared, &fallback, policy, line, "shutdown during partial write").await;
                    } else if let Ok(mut state) = shared.lock() {
                        state.stats.sent += 1;
                    }
                    drain_unsent(&mut receiver, &shared, &fallback, policy).await;
                    break 'writer;
                }
            }
        }
        if let Some(error) = write_error {
            tracing::warn!(%error, "collector write failed");
            connection = None;
            reconnect_at = tokio::time::Instant::now() + RECONNECT_BACKOFF;
            if let Ok(mut state) = shared.lock() {
                state.stats.connected = false;
                state.reconnect_at = Some(reconnect_at);
            }
            unavailable(&shared, &fallback, policy, line, "write failed").await;
        } else if offset == line.len() {
            if let Ok(mut state) = shared.lock() {
                state.stats.sent += 1;
            }
        }
    }
    if let Some(mut stream) = connection {
        let _unused = stream.shutdown().await;
    }
}

async fn drain_unsent(
    receiver: &mut mpsc::Receiver<Vec<u8>>,
    shared: &Arc<Mutex<Shared>>,
    fallback: &Fallback,
    policy: UnavailablePolicy,
) {
    while let Ok(line) = receiver.try_recv() {
        unavailable(shared, fallback, policy, line, "shutdown").await;
    }
}

async fn open(address: &CollectorAddress) -> std::io::Result<LocalConnection> {
    match address {
        CollectorAddress::Unix { path } => Ok(LocalConnection::Unix(
            tokio::net::UnixStream::connect(path).await?,
        )),
        CollectorAddress::Tcp { host, port } => Ok(LocalConnection::Tcp(
            tokio::net::TcpStream::connect((host.as_str(), *port)).await?,
        )),
    }
}
