use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// Collects the level and message of every WARN and ERROR event, named as Python's logging names them.
#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<Value>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Logs {
    pub(super) fn records(&self) -> Value {
        Value::Array(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = if *event.metadata().level() == Level::WARN {
            "WARNING"
        } else if *event.metadata().level() == Level::ERROR {
            "ERROR"
        } else {
            return;
        };
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"level": level, "text": message.0}));
    }
}
