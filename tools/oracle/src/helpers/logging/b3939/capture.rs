use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::field::{Field, Visit};
use tracing::subscriber::Subscriber;
use tracing::{Event, Level};
use tracing_subscriber::layer::{Context, Layer};

const REQUIRED_KEY: &str = "DD_API_KEY";

#[derive(Clone, Default)]
pub(super) struct WarningCounter(pub(super) Arc<AtomicUsize>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for WarningCounter {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        if message.0.contains(REQUIRED_KEY) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}
