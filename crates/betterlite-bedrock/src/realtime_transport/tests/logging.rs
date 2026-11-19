use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::realtime_session_state::logging::{
    RealtimeLogging, RealtimeSuccessQueue, SessionInput, SuccessWork,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[derive(Default)]
struct LoggingState {
    details: Mutex<Vec<(String, Json)>>,
    dispatched: Mutex<Vec<Vec<IndexMap<String, Json>>>>,
}

#[derive(Default)]
pub(super) struct FakeLogging {
    state: Arc<LoggingState>,
}

impl FakeLogging {
    pub(super) fn dispatched(&self) -> Vec<Vec<IndexMap<String, Json>>> {
        self.state
            .dispatched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn detail(&self, key: &str) -> Option<Json> {
        self.state
            .details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }
}

impl RealtimeLogging for FakeLogging {
    fn pre_call(&self, _input: &SessionInput<'_>, _api_key: &str) {}

    fn record_model_call_detail(&self, key: &str, value: Json) {
        self.state
            .details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value));
    }

    fn dispatch_success_handlers(&self, messages: &[IndexMap<String, Json>]) -> SuccessWork {
        self.state
            .dispatched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(messages.to_vec());
        Box::pin(async {})
    }
}

#[derive(Default)]
struct QueueState {
    enqueued: AtomicUsize,
}

pub(super) struct FakeQueue {
    accepted: bool,
    state: Arc<QueueState>,
}

impl FakeQueue {
    pub(super) fn new(accepted: bool) -> Self {
        Self {
            accepted,
            state: Arc::new(QueueState {
                enqueued: AtomicUsize::new(0),
            }),
        }
    }

    pub(super) fn enqueued(&self) -> usize {
        self.state.enqueued.load(Ordering::SeqCst)
    }
}

impl RealtimeSuccessQueue for FakeQueue {
    fn enqueue(&self, _work: SuccessWork) -> bool {
        self.state.enqueued.fetch_add(1, Ordering::SeqCst);
        self.accepted
    }
}
