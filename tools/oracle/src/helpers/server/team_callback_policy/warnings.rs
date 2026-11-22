use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// the fixed text of each WARN line; what follows ": " is the cause, which LiteLLM and betterlite word differently
#[derive(Clone, Default)]
pub(super) struct Warnings(pub(super) Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct FixedText(String);

impl Visit for FixedText {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            let mut rendered = format!("{value:?}");
            if let Some(at) = rendered.find(": ") {
                rendered.truncate(at);
            }
            self.0 = rendered;
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() == Level::WARN {
            let mut text = FixedText::default();
            event.record(&mut text);
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(text.0);
        }
    }
}
