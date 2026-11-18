use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// Only the WARN lines the transform emits reach the oracle's value; anything else is dropped
#[derive(Clone, Default)]
pub(super) struct Warnings {
    captured: Arc<Mutex<Vec<String>>>,
}

impl Warnings {
    pub(super) fn messages(&self) -> Vec<String> {
        self.captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut text = Text::default();
        event.record(&mut text);
        self.captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(text.0);
    }
}

#[derive(Default)]
struct Text(String);

impl Visit for Text {
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
