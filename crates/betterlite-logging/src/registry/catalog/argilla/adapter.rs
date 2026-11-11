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
    async fn logger(&self) -> Result<&ArgillaLogger<T, R>, LoggerError> {
        self.connected
            .get_or_try_init(|| async {
                let pending = self
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                    .ok_or_else(|| delivery_error("the Argilla logger is already connecting"))?;
                ArgillaLogger::connect(pending.settings, pending.transport, pending.random)
                    .await
                    .map_err(|failure| delivery_error(&failure.to_string()))
            })
            .await
    }
}

fn delivery_error(message: &str) -> LoggerError {
    LoggerError::Delivery {
        callback: "argilla".to_owned(),
        message: message.to_owned(),
    }
}

#[async_trait]
impl<T: Transport + 'static, R: RandomSource + 'static> CallbackLogger for ArgillaAdapter<T, R> {
    fn callback_name(&self) -> &'static str {
        "argilla"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let payload = Arc::clone(&event.payload);
        let connection = Arc::clone(&self.connection);
        let _spawned = self.tracker.spawn(async move {
            if let Err(failure) = sync_success(&connection, &payload).await {
                tracing::warn!(%failure, "the Argilla sync success event could not be logged");
            }
        });
        Ok(())
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let logger = self.connection.logger().await?;
        let kwargs = CallbackKwargs {
            standard_logging_object: Some(ExtractionPayload {
                messages: event.payload.messages.clone(),
                response: event.payload.response.clone(),
            }),
        };
        // no other merged callback implements async_dataset_hook, so LiteLLM's default keeps every item
        logger.async_log_success_event(&kwargs, &[]).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let logger = self.connection.logger().await?;
        let kwargs = CallbackKwargs {
            standard_logging_object: Some(ExtractionPayload {
                messages: event.payload.messages.clone(),
                response: event.payload.response.clone(),
            }),
        };
        logger.async_log_failure_event(&kwargs).await;
        Ok(())
    }
}

async fn sync_success<T, R>(
    connection: &Connection<T, R>,
    payload: &StandardLoggingPayload,
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
    logger.log_success_event(&kwargs).await;
    Ok(())
}
