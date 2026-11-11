pub mod error;
pub mod ports;
pub mod session;
pub mod spend_logger;

use std::error::Error;
use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::identity::CallbackIdentity;
use betterlite_logging::registry::phases::CallbackList;
use betterlite_logging::registry::sink::CallbackSink;
use indexmap::IndexMap;

use error::{ReceiverFailure, StartupFailure, TraceLifecycleError};
use ports::{ReceiverFactory, ReceiverHandle};
use session::{Started, TraceSession};
use spend_logger::{CALLBACK_NAME, SpendLoggerFactory, TraceSpendLogger};

// B189 has no async lists: its Success and Failure lists carry both the sync and the async dispatch
pub(crate) const REGISTRATION_LISTS: [CallbackList; 5] = [
    CallbackList::Callbacks,
    CallbackList::Success,
    CallbackList::Failure,
    CallbackList::Success,
    CallbackList::Failure,
];

pub async fn start<Factory, Loggers>(
    enabled: bool,
    factory: &Factory,
    loggers: &Loggers,
    registry: &mut CallbackRegistry,
) -> Result<Option<Started<Factory::Receiver>>, TraceLifecycleError<Factory::Error, Loggers::Error>>
where
    Factory: ReceiverFactory,
    Factory::Receiver: ReceiverHandle<Error = Factory::Error>,
    Loggers: SpendLoggerFactory<<Factory::Receiver as ReceiverHandle>::Storage>,
{
    if !enabled {
        return Ok(None);
    }
    let Some(receiver) = started_receiver(factory)
        .await
        .map_err(|source| TraceLifecycleError::Receiver { source })?
    else {
        return Ok(None);
    };
    let spend = loggers
        .build(receiver.storage())
        .map_err(|source| TraceLifecycleError::SpendLogger { source })?;
    let logger = Arc::new(TraceSpendLogger {
        spend,
        lens: loggers.lens(),
    });
    let identity = CallbackIdentity {
        class_name: CALLBACK_NAME.to_owned(),
        attributes: IndexMap::new(),
    };
    let shared = Arc::clone(&logger);
    let erased: Arc<dyn CallbackLogger> = shared;
    let sink = CallbackSink::Logger(erased);
    for list in REGISTRATION_LISTS {
        let _registration = registry.register(identity.clone(), sink.clone(), list);
    }
    tracing::info!("Agent tracing enabled (store=clickhouse)");
    Ok(Some(Started {
        receiver: Arc::new(receiver),
        session: TraceSession { logger, identity },
    }))
}

async fn started_receiver<Factory>(
    factory: &Factory,
) -> Result<Option<Factory::Receiver>, Factory::Error>
where
    Factory: ReceiverFactory,
    Factory::Receiver: ReceiverHandle<Error = Factory::Error>,
{
    let receiver = match factory.build() {
        Ok(receiver) => receiver,
        Err(source) => return unavailable(source),
    };
    match receiver.start().await {
        Ok(()) => Ok(Some(receiver)),
        Err(source) => unavailable(source),
    }
}

fn unavailable<Receiver, ReceiverError>(
    source: ReceiverError,
) -> Result<Option<Receiver>, ReceiverError>
where
    ReceiverError: ReceiverFailure,
{
    match source.startup_failure() {
        StartupFailure::MissingSetting
        | StartupFailure::Storage
        | StartupFailure::Runtime
        | StartupFailure::InvalidValue => {
            tracing::warn!(error = %error_text(&source), "Agent tracing unavailable");
            Ok(None)
        }
        StartupFailure::Unexpected => Err(source),
    }
}

// LiteLLM's warning prints the exception's str, so a wrapped cause has to reach the log line too
fn error_text(source: &dyn Error) -> String {
    let mut text = source.to_string();
    let mut cause = source.source();
    while let Some(inner) = cause {
        text.push_str(": ");
        text.push_str(&inner.to_string());
        cause = inner.source();
    }
    text
}

mod tests;
