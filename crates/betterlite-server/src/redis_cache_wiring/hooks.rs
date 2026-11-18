use std::error::Error as StdError;
use std::io;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread;

use betterlite_logging::logging_service_events::sink::ServiceEventSink;
use betterlite_logging::logging_service_events::types::{
    ServiceEvent as LoggedServiceEvent, ServiceLoggerPayload, ServiceType,
};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use chrono::TimeDelta;

// The caller supplies this, so a test picks the ids and the hook reads no random source.
pub type CallIdGenerator = Arc<dyn Fn() -> String + Send + Sync>;

// A stalled sink may delay these events but never a cache call, because a full queue drops them.
pub(crate) const EVENTS_QUEUED: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ServiceEventHooksError {
    #[error("the redis service event worker could not start: {source}")]
    Spawn {
        #[source]
        source: io::Error,
    },
}

// Every cache shares this; the sink runs on the worker thread, never on the cache's caller.
pub struct ServiceEventHooks {
    sender: SyncSender<LoggedServiceEvent>,
    clock: Arc<dyn SuccessClock>,
    call_ids: CallIdGenerator,
}

impl ServiceEventHooks {
    pub fn start<S>(
        sink: Arc<S>,
        clock: Arc<dyn SuccessClock>,
        call_ids: CallIdGenerator,
    ) -> Result<Self, ServiceEventHooksError>
    where
        S: ServiceEventSink + 'static,
    {
        let (sender, events) = sync_channel(EVENTS_QUEUED);
        // the handle is dropped at once: the worker stops when the hooks drop the sender, and joining it could hang on a stalled sink
        let _worker = thread::Builder::new()
            .name("redis-service-events".to_owned())
            .spawn(move || deliver(sink, events))
            .map_err(|source| ServiceEventHooksError::Spawn { source })?;
        Ok(Self {
            sender,
            clock,
            call_ids,
        })
    }

    fn emit(&self, event: &ServiceEvent, error: Option<String>) {
        let end_time = self.clock.now();
        let elapsed = TimeDelta::from_std(event.duration).unwrap_or(TimeDelta::MAX);
        let is_error = error.is_some();
        let logged = LoggedServiceEvent {
            call_id: (self.call_ids)(),
            payload: ServiceLoggerPayload {
                is_error,
                error,
                service: ServiceType::Redis,
                duration: event.duration.as_secs_f64(),
                call_type: event.call_type.to_owned(),
                caller: None,
                event_metadata: None,
            },
            start_time: end_time - elapsed,
            end_time,
        };
        if let Err(error) = self.sender.try_send(logged) {
            let reason = match error {
                TrySendError::Full(_dropped) => "the redis service event queue is full",
                TrySendError::Disconnected(_dropped) => "the redis service event worker is gone",
            };
            tracing::warn!(reason, "the redis service event was dropped");
        }
    }
}

impl ServiceHooks for ServiceEventHooks {
    fn success(&self, event: &ServiceEvent) {
        self.emit(event, None);
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn StdError + Send + Sync)) {
        self.emit(event, Some(error.to_string()));
    }
}

fn deliver<S: ServiceEventSink>(sink: Arc<S>, events: Receiver<LoggedServiceEvent>) {
    while let Ok(event) = events.recv() {
        if let Err(error) = sink.emit(&event) {
            tracing::warn!(%error, "the redis service event was not emitted");
        }
    }
}
