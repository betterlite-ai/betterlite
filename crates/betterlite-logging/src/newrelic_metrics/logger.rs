use std::collections::VecDeque;
use std::mem::take;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Handle;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use crate::batch::logger::BatchSettings;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::generic_api_logger::generic_logger::retry::RetrySleeper;

use super::NewRelicMetricsError;
use super::clock::Clock;
use super::drain::drain_with_retry;
use super::flush::{flush_queue, periodic_flush};
use super::queue::{QueuedRecord, push_record};
use super::records::metric_record_from_payload;
use super::region::resolve_metric_endpoint;
use super::transport::NewRelicTransport;

pub const MAX_BATCH_SIZE: usize = 200;
pub const MAX_RETRY_QUEUE: usize = 10_000;

pub struct NewRelicDeps {
    pub transport: Arc<dyn NewRelicTransport>,
    pub clock: Clock,
    pub sleeper: Arc<dyn RetrySleeper>,
    pub settings: BatchSettings,
}

pub struct NewRelicMetricsLogger {
    pub(super) api_key: SecretString,
    pub(super) metric_api_url: &'static str,
    pub(super) transport: Arc<dyn NewRelicTransport>,
    pub(super) clock: Clock,
    pub(super) sleeper: Arc<dyn RetrySleeper>,
    pub(super) settings: BatchSettings,
    pub(super) queue: Mutex<VecDeque<QueuedRecord>>,
    pub(super) next_id: AtomicU64,
    pub(super) flush_lock: AsyncMutex<()>,
    pub(super) drain_lock: AsyncMutex<()>,
    pub(super) last_flush_time: Mutex<Duration>,
    pub(super) stopped: AtomicBool,
    pub(super) flush_task: Mutex<Option<JoinHandle<()>>>,
    pub(super) drains: Mutex<Vec<JoinHandle<()>>>,
}

fn build_logger(
    api_key: SecretString,
    region: Option<&str>,
    deps: NewRelicDeps,
) -> Result<NewRelicMetricsLogger, NewRelicMetricsError> {
    if api_key.expose_secret().is_empty() {
        return Err(NewRelicMetricsError::MissingApiKey);
    }
    if deps.settings.batch_size == 0 {
        return Err(NewRelicMetricsError::ZeroBatchSize);
    }
    let last_flush_time = (deps.clock)();
    Ok(NewRelicMetricsLogger {
        api_key,
        metric_api_url: resolve_metric_endpoint(region),
        transport: deps.transport,
        clock: deps.clock,
        sleeper: deps.sleeper,
        settings: deps.settings,
        queue: Mutex::new(VecDeque::new()),
        next_id: AtomicU64::new(0),
        flush_lock: AsyncMutex::new(()),
        drain_lock: AsyncMutex::new(()),
        last_flush_time: Mutex::new(last_flush_time),
        stopped: AtomicBool::new(false),
        flush_task: Mutex::new(None),
        drains: Mutex::new(Vec::new()),
    })
}

fn spawn_periodic_flush(owner: &Arc<NewRelicMetricsLogger>) -> Result<(), NewRelicMetricsError> {
    let handle = Handle::try_current().map_err(NewRelicMetricsError::NoRuntime)?;
    let weak = Arc::downgrade(owner);
    let task = handle.spawn(async move {
        let Some(logger) = Weak::upgrade(&weak) else {
            return;
        };
        periodic_flush(logger).await;
    });
    *owner
        .flush_task
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(task);
    Ok(())
}

impl NewRelicMetricsLogger {
    pub fn new(
        api_key: SecretString,
        region: Option<&str>,
        deps: NewRelicDeps,
    ) -> Result<Arc<Self>, NewRelicMetricsError> {
        let logger = Arc::new(build_logger(api_key, region, deps)?);
        spawn_periodic_flush(&logger)?;
        Ok(logger)
    }

    // LiteLLM cannot schedule the final drain when no event loop runs, so stop reports that it could not
    pub fn stop(self: &Arc<Self>) -> bool {
        self.stopped.store(true, Ordering::SeqCst);
        let Ok(handle) = Handle::try_current() else {
            tracing::debug!("New Relic Metrics: could not schedule final drain on stop()");
            return false;
        };
        let logger = Arc::clone(self);
        let scheduled = catch_unwind(AssertUnwindSafe(|| {
            handle.spawn(async move {
                drain_with_retry(&logger).await;
            })
        }));
        let Ok(task) = scheduled else {
            tracing::debug!("New Relic Metrics: could not schedule final drain on stop()");
            return false;
        };
        self.drains
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(task);
        true
    }

    pub async fn wait_for_final_drains(&self) {
        let tasks: Vec<JoinHandle<()>> =
            take(&mut *self.drains.lock().unwrap_or_else(PoisonError::into_inner));
        for task in tasks {
            let _outcome = task.await;
        }
    }

    pub async fn record_event(&self, payload: &StandardLoggingPayload) {
        let record = match metric_record_from_payload(payload) {
            Ok(record) => record,
            Err(error) => {
                tracing::error!(%error, "New Relic Metrics Layer Error");
                return;
            }
        };
        let depth = push_record(self, record);
        if self.stopped.load(Ordering::SeqCst) {
            // A stopped logger has no periodic loop left; this callback delivers its own record
            drain_with_retry(self).await;
            return;
        }
        if depth >= self.settings.batch_size {
            flush_queue(self).await;
        }
    }
}

#[async_trait]
impl CallbackLogger for NewRelicMetricsLogger {
    fn callback_name(&self) -> &'static str {
        "newrelic"
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record_event(event.payload.as_ref()).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record_event(event.payload.as_ref()).await;
        Ok(())
    }
}
