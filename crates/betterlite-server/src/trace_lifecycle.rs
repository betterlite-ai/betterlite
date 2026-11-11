pub mod error;
pub mod ports;
pub mod session;
pub mod spend_logger;

use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::identity::CallbackIdentity;
use betterlite_logging::registry::phases::CallbackList;
use betterlite_logging::registry::sink::CallbackSink;
use indexmap::IndexMap;

use error::TraceLifecycleError;
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
) -> Result<Option<Started<Factory::Receiver>>, TraceLifecycleError>
where
    Factory: ReceiverFactory,
    Loggers: SpendLoggerFactory<<Factory::Receiver as ReceiverHandle>::Storage>,
{
    if !enabled {
        return Ok(None);
    }
    let Some(receiver) = started_receiver(factory).await? else {
        return Ok(None);
    };
    let logger = Arc::new(TraceSpendLogger {
        spend: loggers.build(receiver.storage())?,
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
) -> Result<Option<Factory::Receiver>, TraceLifecycleError>
where
    Factory: ReceiverFactory,
{
    let receiver = match factory.build() {
        Ok(receiver) => receiver,
        Err(error) => return unavailable(error),
    };
    match receiver.start().await {
        Ok(()) => Ok(Some(receiver)),
        Err(error) => unavailable(error),
    }
}

fn unavailable<Receiver>(
    error: TraceLifecycleError,
) -> Result<Option<Receiver>, TraceLifecycleError> {
    match error {
        TraceLifecycleError::MissingSetting { .. }
        | TraceLifecycleError::Storage { .. }
        | TraceLifecycleError::Runtime { .. }
        | TraceLifecycleError::InvalidValue { .. } => {
            tracing::warn!(error = %error, "Agent tracing unavailable");
            Ok(None)
        }
        TraceLifecycleError::Unexpected { .. } => Err(error),
    }
}

mod tests;
