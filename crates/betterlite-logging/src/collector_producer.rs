use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::collector::{
    CollectorAddress, CollectorSettings, UnavailablePolicy, parse_collector_address,
};
use tokio::sync::mpsc;

mod writer;

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

pub(super) struct Shared {
    pub(super) stats: ProducerStats,
    pub(super) closing: bool,
    pub(super) reconnect_at: Option<tokio::time::Instant>,
}

pub struct SpendEventProducer {
    sender: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    shared: Arc<Mutex<Shared>>,
    fallback: Fallback,
    policy: UnavailablePolicy,
    shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    writer: Mutex<Option<tokio::task::JoinHandle<()>>>,
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
        let (sender, receiver) = mpsc::channel(capacity.max(1));
        let (shutdown, shutdown_receiver) = tokio::sync::oneshot::channel();
        let shared = Arc::new(Mutex::new(Shared {
            stats: ProducerStats::default(),
            closing: false,
            reconnect_at: None,
        }));
        let worker_shared = Arc::clone(&shared);
        let worker_fallback = Arc::clone(&fallback);
        let connect_timeout =
            Duration::try_from_secs_f64(connect_timeout_seconds).unwrap_or_default();
        let writer = tokio::spawn(writer::run_writer(
            address,
            receiver,
            shutdown_receiver,
            worker_shared,
            worker_fallback,
            policy,
            connect_timeout,
        ));
        Self {
            sender: Mutex::new(Some(sender)),
            shared,
            fallback,
            policy,
            shutdown: Mutex::new(Some(shutdown)),
            writer: Mutex::new(Some(writer)),
        }
    }

    pub async fn publish(&self, line: Vec<u8>) -> PublishOutcome {
        let now = tokio::time::Instant::now();
        let unavailable_reason = self
            .shared
            .lock()
            .map(|state| {
                if state.closing {
                    Some("closing")
                } else if state
                    .reconnect_at
                    .is_some_and(|reconnect_at| now < reconnect_at)
                {
                    Some("sidecar reconnect backoff")
                } else {
                    None
                }
            })
            .unwrap_or(Some("producer unavailable"));
        if let Some(reason) = unavailable_reason {
            return unavailable(&self.shared, &self.fallback, self.policy, line, reason).await;
        }
        let sender = self
            .sender
            .lock()
            .ok()
            .and_then(|sender| sender.as_ref().cloned());
        let Some(sender) = sender else {
            return unavailable(&self.shared, &self.fallback, self.policy, line, "closing").await;
        };
        match sender.try_send(line) {
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

    pub async fn close(&self, drain_timeout: Duration) {
        if let Ok(mut state) = self.shared.lock() {
            state.closing = true;
        }
        if let Ok(mut sender) = self.sender.lock() {
            sender.take();
        }
        let writer = self.writer.lock().ok().and_then(|mut writer| writer.take());
        let Some(mut writer) = writer else {
            return;
        };
        if tokio::time::timeout(drain_timeout, &mut writer)
            .await
            .is_err()
        {
            if let Ok(mut shutdown) = self.shutdown.lock() {
                if let Some(shutdown) = shutdown.take() {
                    let _ = shutdown.send(());
                }
            }
            let _unused = writer.await;
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

#[path = "collector_producer/tests.rs"]
mod producer_tests;
