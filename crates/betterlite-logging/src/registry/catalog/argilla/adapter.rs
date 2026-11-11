use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use tokio::sync::OnceCell;
use tokio_util::task::TaskTracker;

use crate::argilla_logging::extraction::{
    CallbackKwargs, StandardLoggingPayload as ExtractionPayload,
};
use crate::argilla_logging::logger::{ArgillaLogger, RandomSource};
use crate::argilla_logging::settings::ArgillaSettings;
use crate::argilla_logging::transport::Transport;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;

pub(super) struct Pending<T, R> {
    pub(super) settings: ArgillaSettings,
    pub(super) transport: T,
    pub(super) random: R,
}

pub(super) struct Connection<T, R> {
    pub(super) connected: OnceCell<ArgillaLogger<T, R>>,
    pending: Mutex<Option<Pending<T, R>>>,
}

pub(super) struct ArgillaAdapter<T, R> {
    pub(super) connection: Arc<Connection<T, R>>,
    pub(super) tracker: TaskTracker,
}

impl<T, R> ArgillaAdapter<T, R> {
    pub(super) fn new(pending: Pending<T, R>, tracker: TaskTracker) -> Self {
        Self {
            connection: Arc::new(Connection {
                connected: OnceCell::new(),
                pending: Mutex::new(Some(pending)),
            }),
            tracker,
        }
    }
}

impl<T: Transport, R: RandomSource> Connection<T, R> {
    // LiteLLM resolves the dataset once in ArgillaLogger.__init__; betterlite builds the logger on the first event it receives.
    pub(super) async fn logger(&self) -> Result<&ArgillaLogger<T, R>, LoggerError> {
        self.connected
            .get_or_try_init(|| async {
                let pending = self
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                    .ok_or_else(|| delivery_error("the Argilla logger is not connected"))?;
                ArgillaLogger::connect(pending.settings, pending.transport, pending.random)
                    .await
                    .map_err(|failure| delivery_error(&failure.to_string()))
            })
            .await
    }
}

#[derive(Clone, Copy)]
enum ArgillaCall {
    SyncSuccess,
    AsyncSuccess,
    AsyncFailure,
}

fn delivery_error(message: &str) -> LoggerError {
    LoggerError::Delivery {
        callback: "argilla".to_owned(),
        message: message.to_owned(),
    }
}

// LiteLLM has a sync success hook and an async success hook that also runs the dataset hooks; no merged callback implements one, so the list stays empty.
#[async_trait]
impl<T: Transport + 'static, R: RandomSource + 'static> CallbackLogger for ArgillaAdapter<T, R> {
    fn callback_name(&self) -> &'static str {
        "argilla"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let payload = Arc::clone(&event.payload);
        let connection = Arc::clone(&self.connection);
        let _spawned = self.tracker.spawn(async move {
            if let Err(failure) = log(&connection, &payload, ArgillaCall::SyncSuccess).await {
                tracing::warn!(%failure, "the Argilla sync success event could not be logged");
            }
        });
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        log(&self.connection, &event.payload, ArgillaCall::AsyncSuccess).await
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        log(&self.connection, &event.payload, ArgillaCall::AsyncFailure).await
    }
}

async fn log<T, R>(
    connection: &Connection<T, R>,
    payload: &StandardLoggingPayload,
    call: ArgillaCall,
) -> Result<(), LoggerError>
where
    T: Transport,
    R: RandomSource,
{
    let logger = connection.logger().await?;
    let kwargs = CallbackKwargs {
        standard_logging_object: Some(ExtractionPayload {
            messages: payload.messages.clone(),
            response: payload.response.clone(),
        }),
    };
    match call {
        ArgillaCall::SyncSuccess => logger.log_success_event(&kwargs).await,
        ArgillaCall::AsyncSuccess => logger.async_log_success_event(&kwargs, &[]).await,
        ArgillaCall::AsyncFailure => logger.async_log_failure_event(&kwargs).await,
    }
    Ok(())
}
