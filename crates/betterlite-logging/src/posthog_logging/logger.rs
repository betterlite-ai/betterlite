use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use thiserror::Error;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::logger::BatchQueue;
use crate::callback_logger::event::LoggerEvent;
use crate::callback_logger::{CallbackLogger, error::LoggerError};

use super::batch::{self, QueuedEvent};
use super::credentials::{self, CredentialError};
use super::delivery::Transport;
use super::event::{self, IdGenerator, PostHogEventError};
use super::exit::spawn_exit_flush;
use super::settings::PostHogSettings;
use super::sink::PostHogSink;
use crate::finops::pointfive::upload::url_check::UploadUrlCheck;

#[derive(Debug, Error)]
pub enum PostHogLayerError {
    #[error(transparent)]
    Credentials(#[from] CredentialError),
    #[error(transparent)]
    Event(#[from] PostHogEventError),
}

pub struct PostHogDeps {
    pub transport: Arc<dyn Transport>,
    pub url_check: Arc<dyn UploadUrlCheck>,
    pub ids: IdGenerator,
    pub tasks: TaskTracker,
    // the server's shutdown token; a logger built without one runs no exit flush
    pub shutdown: Option<CancellationToken>,
}

pub struct PostHogLogger {
    pub settings: PostHogSettings,
    pub sink: PostHogSink,
    pub ids: IdGenerator,
    pub tasks: TaskTracker,
    pub stopped: CancellationToken,
    pub async_initialized: AtomicBool,
}

impl PostHogLogger {
    pub fn new(settings: PostHogSettings, deps: PostHogDeps) -> Arc<Self> {
        let stopped = deps.shutdown.clone().unwrap_or_default();
        let sink = PostHogSink {
            queue: BatchQueue::new(settings.batch),
            transport: deps.transport,
            url_check: deps.url_check,
            mock_mode: settings.mock_mode,
        };
        let logger = Arc::new(Self {
            settings,
            sink,
            ids: deps.ids,
            tasks: deps.tasks,
            stopped,
            async_initialized: AtomicBool::new(false),
        });
        if let Some(shutdown) = deps.shutdown {
            let registered = spawn_exit_flush(logger.sink.clone(), shutdown, &logger.tasks);
            if !registered {
                tracing::debug!("PostHog: no runtime, the exit flush did not register");
            }
        }
        logger
    }
    pub fn queued_event(&self, event: &LoggerEvent) -> Result<QueuedEvent, PostHogLayerError> {
        let credentials =
            credentials::credentials_for_request(&self.settings, &event.dynamic_params)?;
        let payload = event::create_event_payload(&event.payload, self.ids)?;
        Ok(QueuedEvent {
            event: payload,
            api_key: credentials.api_key,
            api_url: credentials.api_url,
            request_url: credentials.request_url,
        })
    }

    // PostHogLogger._ensure_async_setup: the flush lock and the periodic flush are made on the first async event
    pub fn ensure_async_setup(&self) -> bool {
        if self.async_initialized.swap(true, Ordering::SeqCst) {
            return true;
        }
        let Ok(handle) = Handle::try_current() else {
            self.async_initialized.store(false, Ordering::SeqCst);
            tracing::error!("PostHog: Failed to initialize async components: no runtime");
            return false;
        };
        let _task = self.tasks.spawn_on(
            batch::periodic_flush(
                self.sink.clone(),
                self.settings.batch.flush_interval,
                self.stopped.clone(),
                self.tasks.clone(),
            ),
            &handle,
        );
        tracing::debug!("PostHog: Async components initialized");
        true
    }

    pub async fn flush_queue(&self) {
        if !self.async_initialized.load(Ordering::SeqCst) {
            return;
        }
        tracing::debug!("PostHog: Flushing batch of events");
        self.sink.flush().await;
    }
}

#[async_trait]
impl CallbackLogger for PostHogLogger {
    fn callback_name(&self) -> &'static str {
        "posthog"
    }
    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let queued = match self.queued_event(event) {
            Ok(queued) => queued,
            Err(error) => {
                tracing::error!(%error, "PostHog Sync Layer Error");
                return Ok(());
            }
        };
        let Ok(handle) = Handle::try_current() else {
            tracing::debug!("PostHog: no runtime, the sync event was not sent");
            return Ok(());
        };
        let sink = self.sink.clone();
        let _task = self
            .tasks
            .spawn_on(async move { sink.send_one(queued).await }, &handle);
        Ok(())
    }
    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.ensure_async_setup();
        batch::log_async_event(self, &event).await;
        Ok(())
    }
    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.ensure_async_setup();
        batch::log_async_event(self, &event).await;
        Ok(())
    }
}
