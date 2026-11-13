use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

const NON_BLOCKING: &str = "LiteLLM.LoggingError: [Non-Blocking] Exception occurred while logging";

#[derive(Clone, Default)]
pub(super) struct ErrorLines(pub(super) Arc<AtomicUsize>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for ErrorLines {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        if message.0.starts_with(NON_BLOCKING) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}
