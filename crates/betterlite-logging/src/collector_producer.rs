use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::collector::{
    AddressError, CollectorSettings, SettingsError, UnavailablePolicy, parse_collector_address,
};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

mod tests;
mod transport;
mod writer;

#[derive(Debug, thiserror::Error)]
pub enum ProducerError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Address(#[from] AddressError),
    #[error("publish cancelled after fallback completed")]
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub enum FallbackError {
    #[error("in-process spend callback failed")]
    Callback {
        #[source]
        source: io::Error,
    },
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

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

struct State {
    stats: ProducerStats,
    sender: Option<mpsc::Sender<Vec<u8>>>,
    reconnect_at: Option<Instant>,
}

impl State {
    fn admit(&mut self, line: Vec<u8>, now: Instant) -> Result<(), (Vec<u8>, &'static str)> {
        if self.reconnect_at.is_some_and(|at| now < at) {
            return Err((line, "reconnect backoff"));
        }
        let Some(sender) = &self.sender else {
            return Err((line, "closing"));
        };
        sender
            .try_send(line)
            .map_err(|error| (error.into_inner(), "queue unavailable"))?;
        self.stats.queued = self.stats.queued.saturating_add(1);
        Ok(())
    }
}

struct Outage {
    state: Mutex<State>,
    fallback: Fallback,
    policy: UnavailablePolicy,
    tasks: TaskTracker,
    clock: Arc<dyn Clock>,
}

impl Outage {
    async fn apply(&self, line: Vec<u8>, reason: &'static str) -> PublishOutcome {
        if self.policy == UnavailablePolicy::Drop {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.stats.dropped = state.stats.dropped.saturating_add(1);
            if state.stats.dropped.checked_rem(1000) == Some(1) {
                tracing::warn!(
                    reason,
                    dropped = state.stats.dropped,
                    "dropping collector spend event"
                );
            }
            return PublishOutcome::Dropped;
        }
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.stats.fallback = state.stats.fallback.saturating_add(1);
        };
        let fallback = Arc::clone(&self.fallback);
        let work = self.tasks.spawn(async move {
            if let Err(error) = fallback(line).await {
                tracing::error!(%error, reason, "collector fallback failed");
            }
        });
        if let Err(error) = work.await {
            tracing::error!(%error, "collector fallback task failed");
        }
        PublishOutcome::Fallback
    }
}

pub struct SpendEventProducer {
    outage: Arc<Outage>,
    shutdown: CancellationToken,
    finished: oneshot::Receiver<()>,
    drain_timeout: Duration,
}

impl SpendEventProducer {
    pub fn build(
        settings: &CollectorSettings,
        fallback: Fallback,
        tasks: &TaskTracker,
    ) -> Result<Option<Self>, ProducerError> {
        settings.validate()?;
        if !settings.enabled || settings.job_role.as_deref() == Some("collector") {
            return Ok(None);
        }
        match parse_collector_address(&settings.address) {
            Ok(address) => {
                tracing::info!(
                    buffer = settings.buffer_size,
                    "offloading spend to collector"
                );
                Self::start(
                    settings,
                    fallback,
                    tasks,
                    transport::Network { address },
                    SystemClock,
                )
                .map(Some)
            }
            Err(error @ AddressError::Syntax(_)) => Err(ProducerError::Address(error)),
            Err(error @ (AddressError::Invalid | AddressError::RemoteHost)) => {
                tracing::error!(%error, "keeping spend tracking in process");
                Ok(None)
            }
        }
    }

    fn start<Connect, Clk>(
        settings: &CollectorSettings,
        fallback: Fallback,
        tasks: &TaskTracker,
        connect: Connect,
        clock: Clk,
    ) -> Result<Self, ProducerError>
    where
        Connect: transport::Connector,
        Clk: Clock + 'static,
    {
        let (drain_timeout, connect_timeout) = settings.validate()?;
        let (sender, receiver) = mpsc::channel(settings.buffer_size);
        let (done, finished) = oneshot::channel();
        let outage = Arc::new(Outage {
            state: Mutex::new(State {
                stats: ProducerStats::default(),
                sender: Some(sender),
                reconnect_at: None,
            }),
            fallback,
            policy: settings.on_unavailable,
            tasks: tasks.clone(),
            clock: Arc::new(clock),
        });
        let shutdown = CancellationToken::new();
        let worker = writer::Writer {
            outage: Arc::clone(&outage),
            shutdown: shutdown.clone(),
            connect_timeout,
        };
        let _writer = tasks.spawn(async move {
            worker.run(receiver, connect).await;
            let _sent = done.send(());
        });
        Ok(Self {
            outage,
            shutdown,
            finished,
            drain_timeout,
        })
    }

    pub async fn publish(
        &self,
        line: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<PublishOutcome, ProducerError> {
        let now = self.outage.clock.now();
        let unsent = self
            .outage
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(line, now);
        match unsent {
            Ok(()) => Ok(PublishOutcome::Queued),
            Err((event, reason)) => {
                let outcome = self.outage.apply(event, reason).await;
                if cancel.is_cancelled() {
                    Err(ProducerError::Cancelled)
                } else {
                    Ok(outcome)
                }
            }
        }
    }

    #[must_use]
    pub fn stats(&self) -> ProducerStats {
        self.outage
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stats
    }

    pub async fn close(&mut self) {
        if self.finished.is_terminated() {
            return;
        }
        self.outage
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sender
            .take();
        if timeout(self.drain_timeout, &mut self.finished)
            .await
            .is_err()
        {
            tracing::warn!("collector drain timeout; applying outage policy");
            self.shutdown.cancel();
            let _finished = (&mut self.finished).await;
        }
    }
}

impl Drop for SpendEventProducer {
    fn drop(&mut self) {
        self.outage
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sender
            .take();
        self.shutdown.cancel();
    }
}
