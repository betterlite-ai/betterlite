pub mod connection;
pub mod delivery;
pub mod error;
pub mod params;
pub mod payload;
pub mod queue;
mod tests;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::logger::{BatchQueue, BatchSettings, DEFAULT_MAX_QUEUE_SIZE};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::zerobus::client::Clock;
use crate::zerobus::client::init_params::ZerobusInitParams;
use crate::zerobus::row::{TraceRow, trace_row};

use connection::{ConnectionError, SecretSource, connection_for};
use delivery::{ClientDelivery, ClientFactory, RowIngest, same_connection};
use error::EnqueueError;
use payload::{PayloadPolicy, RedactionSettings, event_details, payload_for};
use queue::{FlushFlag, Inflight};

pub const CALLBACK_NAME: &str = "zerobus";

pub struct ZerobusDeps {
    pub params: ZerobusInitParams,
    pub client: Option<Arc<dyn RowIngest>>,
    pub secrets: Arc<dyn SecretSource>,
    pub http: ProxiedClient,
    pub clock: Clock,
    pub redaction: RedactionSettings,
    pub tasks: TaskTracker,
    pub shutdown: CancellationToken,
    pub start_periodic_flush: bool,
    pub max_queue_size: Option<usize>,
}

pub struct ZerobusLogger {
    inner: Arc<Inner>,
    tasks: TaskTracker,
    shutdown: CancellationToken,
    periodic: Mutex<Option<JoinHandle<()>>>,
    batch: Mutex<Option<JoinHandle<()>>>,
}

pub(super) struct Inner {
    params: ZerobusInitParams,
    given_client: Option<Arc<dyn RowIngest>>,
    cached_client: Mutex<Option<Arc<dyn RowIngest>>>,
    factory: ClientFactory,
    secrets: Arc<dyn SecretSource>,
    redaction: RedactionSettings,
    queue: BatchQueue<TraceRow>,
    flushing: AtomicBool,
    inflight: AtomicUsize,
}

impl ZerobusLogger {
    pub fn new(deps: ZerobusDeps) -> Result<Self, ConnectionError> {
        if deps.client.is_none() {
            connection_for(&deps.params, deps.secrets.as_ref())?;
        }
        let settings = batch_settings(&deps.params, deps.max_queue_size);
        let inner = Arc::new(Inner {
            params: deps.params,
            given_client: deps.client,
            cached_client: Mutex::new(None),
            factory: ClientFactory::new(deps.http, deps.clock),
            secrets: deps.secrets,
            redaction: deps.redaction,
            queue: BatchQueue::new(settings),
            flushing: AtomicBool::new(false),
            inflight: AtomicUsize::new(0),
        });
        let logger = Self {
            inner,
            tasks: deps.tasks,
            shutdown: deps.shutdown.child_token(),
            periodic: Mutex::new(None),
            batch: Mutex::new(None),
        };
        if deps.start_periodic_flush {
            let started = logger.spawn_periodic_flush();
            *logger
                .periodic
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = started;
        }
        Ok(logger)
    }

    fn spawn_periodic_flush(&self) -> Option<JoinHandle<()>> {
        if Handle::try_current().is_err() {
            return None;
        }
        let inner = Arc::clone(&self.inner);
        let shutdown = self.shutdown.clone();
        Some(self.tasks.spawn(async move {
            inner.periodic_flush(&shutdown).await;
        }))
    }

    fn start_batch_flush_task(&self) {
        let mut batch = self.batch.lock().unwrap_or_else(PoisonError::into_inner);
        if batch.as_ref().is_some_and(|task| !task.is_finished()) || Handle::try_current().is_err()
        {
            return;
        }
        let inner = Arc::clone(&self.inner);
        let spawned = self.tasks.spawn(async move {
            inner.flush_queue(true).await;
        });
        *batch = Some(spawned);
    }

    fn enqueue(&self, event: &LoggerEvent) {
        let mut periodic = self.periodic.lock().unwrap_or_else(PoisonError::into_inner);
        if periodic.as_ref().is_none_or(JoinHandle::is_finished) {
            *periodic = self.spawn_periodic_flush();
        }
        drop(periodic);
        if let Err(error) = self.try_enqueue(event) {
            tracing::error!(%error, "zerobus: failed to queue an event");
        }
    }

    fn try_enqueue(&self, event: &LoggerEvent) -> Result<(), EnqueueError> {
        let details = event_details(event)?;
        let policy = PayloadPolicy {
            turn_off_message_logging: self.inner.params.turn_off_message_logging,
            excluded_fields: &self.inner.redaction.excluded_fields,
            global_turn_off: self.inner.redaction.turn_off_message_logging,
        };
        let Some(payload) = payload_for(&details, &policy) else {
            tracing::debug!("zerobus: event carried no standard_logging_object, skipping");
            return Ok(());
        };
        if self.inner.at_capacity_during_flush() {
            tracing::warn!(
                max_queue_size = self.inner.queue.settings.max_queue_size,
                "zerobus: queue at its cap during a flush, dropped a row"
            );
            return Ok(());
        }
        let row = trace_row(&payload)?;
        let enqueued = self.inner.queue.enqueue(row);
        if enqueued.depth >= self.inner.queue.settings.batch_size {
            self.start_batch_flush_task();
        }
        Ok(())
    }
}

fn batch_settings(params: &ZerobusInitParams, max_queue_size: Option<usize>) -> BatchSettings {
    BatchSettings {
        batch_size: params.batch_size.get(),
        flush_interval: Duration::from_secs(params.flush_interval.get()),
        max_queue_size: max_queue_size.unwrap_or(DEFAULT_MAX_QUEUE_SIZE),
    }
}

impl Inner {
    fn depth(&self) -> usize {
        self.queue
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn at_capacity_during_flush(&self) -> bool {
        if !self.flushing.load(Ordering::SeqCst) {
            return false;
        }
        let held = self
            .depth()
            .saturating_add(self.inflight.load(Ordering::SeqCst));
        held >= self.queue.settings.max_queue_size
    }

    pub(super) fn client(&self) -> Result<Arc<dyn RowIngest>, ConnectionError> {
        if let Some(given) = &self.given_client {
            return Ok(Arc::clone(given));
        }
        let connection = connection_for(&self.params, self.secrets.as_ref())?;
        let mut cached = self
            .cached_client
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(client) = cached.as_ref()
            && same_connection(client.connection(), &connection)
        {
            return Ok(Arc::clone(client));
        }
        let fresh = self.factory.build(connection);
        *cached = Some(Arc::clone(&fresh));
        drop(cached);
        Ok(fresh)
    }

    // a mid-flush arrival is counted against the rows the delivery holds, so the cap still holds
    pub(super) async fn flush_queue(&self, skip_if_flushing: bool) {
        if skip_if_flushing && self.flushing.load(Ordering::SeqCst) {
            return;
        }
        let _flushing = FlushFlag::set(&self.flushing);
        let pending = self.depth();
        let _inflight = Inflight::set(&self.inflight, pending);
        self.queue.flush(&ClientDelivery { inner: self }).await;
    }

    pub(super) async fn periodic_flush(&self, shutdown: &CancellationToken) {
        loop {
            if timeout(self.queue.settings.flush_interval, shutdown.cancelled())
                .await
                .is_ok()
            {
                return;
            }
            self.flush_queue(false).await;
        }
    }
}

#[async_trait]
impl CallbackLogger for ZerobusLogger {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    fn turn_off_message_logging(&self) -> bool {
        self.inner.params.turn_off_message_logging
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.enqueue(&event);
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.enqueue(&event);
        Ok(())
    }
}

impl Drop for ZerobusLogger {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}
