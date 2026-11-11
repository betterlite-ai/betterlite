use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const EVENTS: [(&str, &str); 3] = [
    (
        "standard_logging_object not found in kwargs",
        "missing_payload",
    ),
    ("exceeded max_queue_size", "queue_overflow"),
    ("Payload too large for a single record", "record_too_large"),
];

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<&'static str>>>);

impl Logs {
    pub(super) fn counts(&self) -> Value {
        let seen = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let counts: BTreeMap<&str, usize> = EVENTS
            .iter()
            .map(|(_, name)| (*name, seen.iter().filter(|logged| *logged == name).count()))
            .collect();
        json!(counts)
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        let mut seen = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for (fragment, name) in EVENTS {
            if message.0.contains(fragment) {
                seen.push(name);
            }
        }
    }
}
