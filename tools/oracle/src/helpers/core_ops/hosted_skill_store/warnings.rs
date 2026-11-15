use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

#[derive(Clone, Default)]
pub(super) struct WarnLog(Arc<Mutex<Vec<Value>>>);

impl WarnLog {
    pub(super) fn entries(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for WarnLog {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != tracing::Level::WARN {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"level": "warning"}));
    }
}
