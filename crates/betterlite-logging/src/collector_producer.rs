use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::collector::{
    CollectorAddress, CollectorSettings, UnavailablePolicy, parse_collector_address,
};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

const RECONNECT_BACKOFF: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
#[error("spend fallback failed: {message}")]
pub struct FallbackError {
    pub message: String,
}

pub type Fallback = Arc<
    dyn Fn(Vec<u8>) -> Pin<Box<dyn Future<Output = Result<(), FallbackError>> + Send>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Queued,
    Fallback,
    Dropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProducerStats {
    pub queued: u64,
    pub sent: u64,
    pub fallback: u64,
    pub dropped: u64,
    pub connected: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ProducerError {
    #[error(transparent)]
    Settings(#[from] betterlite_config::collector::SettingsError),
    #[error("invalid collector address: {0}")]
    Address(#[from] betterlite_config::collector::AddressError),
}

struct Shared {
    stats: ProducerStats,
    closing: bool,
}

pub struct SpendEventProducer {
    sender: mpsc::Sender<Vec<u8>>,
    shared: Arc<Mutex<Shared>>,
    fallback: Fallback,
    policy: UnavailablePolicy,
}

impl SpendEventProducer {
    pub fn build(
        settings: &CollectorSettings,
        fallback: Fallback,
    ) -> Result<Option<Self>, ProducerError> {
        settings.validate()?;
        if !(settings.enabled && settings.job_role.as_deref() != Some("collector")) {
            return Ok(None);
        }
        let address = match parse_collector_address(&settings.address) {
            Ok(address) => address,
            Err(betterlite_config::collector::AddressError::RemoteHost(_)) => {
                tracing::error!(
                    "collector address must use loopback; keeping spend tracking in process"
                );
                return Ok(None);
            }
            Err(error @ betterlite_config::collector::AddressError::Syntax(_)) => {
                return Err(ProducerError::Address(error));
            }
            Err(_) => {
                tracing::error!("invalid collector address; keeping spend tracking in process");
                return Ok(None);
            }
        };
        Ok(Some(Self::new(
            address,
            settings.buffer_size,
            settings.connect_timeout_seconds,
            settings.on_unavailable,
            fallback,
        )))
    }

    pub fn new(
        address: CollectorAddress,
        capacity: usize,
        connect_timeout_seconds: f64,
        policy: UnavailablePolicy,
        fallback: Fallback,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(capacity);
        let shared = Arc::new(Mutex::new(Shared {
            stats: ProducerStats::default(),
            closing: false,
        }));
        let worker_shared = Arc::clone(&shared);
        let worker_fallback = Arc::clone(&fallback);
        let connect_timeout =
            Duration::try_from_secs_f64(connect_timeout_seconds).unwrap_or_default();
        tokio::spawn(run_writer(
            address,
            receiver,
            worker_shared,
            worker_fallback,
            policy,
            connect_timeout,
        ));
        Self {
            sender,
            shared,
            fallback,
            policy,
        }
    }

    pub async fn publish(&self, line: Vec<u8>) -> PublishOutcome {
        let closing = self
            .shared
            .lock()
            .map(|state| state.closing)
            .unwrap_or(true);
        if closing {
            return unavailable(&self.shared, &self.fallback, self.policy, line, "closing").await;
        }
        match self.sender.try_send(line) {
            Ok(()) => {
                if let Ok(mut state) = self.shared.lock() {
                    state.stats.queued += 1;
                }
                PublishOutcome::Queued
            }
            Err(mpsc::error::TrySendError::Full(line)) => {
                unavailable(
                    &self.shared,
                    &self.fallback,
                    self.policy,
                    line,
                    "buffer full",
                )
                .await
            }
            Err(mpsc::error::TrySendError::Closed(line)) => {
                unavailable(
                    &self.shared,
                    &self.fallback,
                    self.policy,
                    line,
                    "writer closed",
                )
                .await
            }
        }
    }

    pub fn stats(&self) -> ProducerStats {
        self.shared
            .lock()
            .map(|state| state.stats)
            .unwrap_or_default()
    }

    pub async fn close(&self, _drain_timeout: Duration) {
        if let Ok(mut state) = self.shared.lock() {
            state.closing = true;
        }
    }
}

async fn unavailable(
    shared: &Arc<Mutex<Shared>>,
    fallback: &Fallback,
    policy: UnavailablePolicy,
    line: Vec<u8>,
    reason: &str,
) -> PublishOutcome {
    match policy {
        UnavailablePolicy::Fallback => {
            if let Ok(mut state) = shared.lock() {
                state.stats.fallback += 1;
            }
            let work = tokio::spawn(fallback(line));
            match work.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!(%error, reason, "collector fallback failed"),
                Err(error) => tracing::error!(%error, reason, "collector fallback task failed"),
            }
            PublishOutcome::Fallback
        }
        UnavailablePolicy::Drop => {
            if let Ok(mut state) = shared.lock() {
                state.stats.dropped += 1;
                if state.stats.dropped % 1000 == 1 {
                    tracing::warn!(reason, "dropping spend event for collector outage");
                }
            }
            PublishOutcome::Dropped
        }
    }
}

async fn run_writer(
    address: CollectorAddress,
    mut receiver: mpsc::Receiver<Vec<u8>>,
    shared: Arc<Mutex<Shared>>,
    fallback: Fallback,
    policy: UnavailablePolicy,
    timeout: Duration,
) {
    let mut connection: Option<Box<dyn AsyncWrite + Send + Unpin>> = None;
    let mut reconnect_at = tokio::time::Instant::now();
    while let Some(line) = receiver.recv().await {
        if connection.is_none() && tokio::time::Instant::now() >= reconnect_at {
            match tokio::time::timeout(timeout, open(&address)).await {
                Ok(Ok(stream)) => {
                    connection = Some(stream);
                    if let Ok(mut state) = shared.lock() {
                        state.stats.connected = true;
                    }
                }
                _ => {
                    reconnect_at = tokio::time::Instant::now() + RECONNECT_BACKOFF;
                    tracing::warn!("collector connection unavailable; entering one second backoff");
                }
            }
        }
        let Some(stream) = connection.as_mut() else {
            unavailable(&shared, &fallback, policy, line, "sidecar unreachable").await;
            continue;
        };
        if let Err(error) = stream.write_all(&line).await {
            tracing::warn!(%error, "collector write failed");
            connection = None;
            reconnect_at = tokio::time::Instant::now() + RECONNECT_BACKOFF;
            if let Ok(mut state) = shared.lock() {
                state.stats.connected = false;
            }
            unavailable(&shared, &fallback, policy, line, "write failed").await;
        } else if let Ok(mut state) = shared.lock() {
            state.stats.sent += 1;
        }
    }
    if let Some(mut stream) = connection {
        let _ = stream.shutdown().await;
    }
}

async fn open(address: &CollectorAddress) -> std::io::Result<Box<dyn AsyncWrite + Send + Unpin>> {
    match address {
        CollectorAddress::Unix { path } => {
            Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
        }
        CollectorAddress::Tcp { host, port } => Ok(Box::new(
            tokio::net::TcpStream::connect((host.as_str(), *port)).await?,
        )),
    }
}

#[path = "collector_producer/tests.rs"]
mod producer_tests;
