use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

const FUNCTION_NAME_MISSING: &str = "name not found for call_id";

// the helper compares labels, so a mapped label replaces the whole message the transform logged
fn label(message: &str) -> String {
    if message.contains(FUNCTION_NAME_MISSING) {
        "function_name_missing".to_owned()
    } else {
        message.to_owned()
    }
}

#[derive(Clone, Default)]
pub(super) struct Warnings {
    labels: Arc<Mutex<Vec<String>>>,
}

impl Warnings {
    pub(super) fn collected(&self) -> Vec<String> {
        self.labels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.labels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(label(&message.0));
    }
}
