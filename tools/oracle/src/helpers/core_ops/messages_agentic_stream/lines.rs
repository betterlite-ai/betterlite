use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use serde_json::Value;
use serde_json::json;
use tracing::Event;
use tracing::Level;
use tracing::Subscriber;
use tracing::field::Field;
use tracing::field::Visit;
use tracing_subscriber::layer::Context;
use tracing_subscriber::layer::Layer;

const PREFIX: &str = "AgenticStreamingIterator:";

#[derive(Clone, Default)]
pub(crate) struct Lines(pub(crate) Arc<Mutex<Vec<Value>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Lines {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        if !message.0.starts_with(PREFIX) {
            return;
        }
        let level = match *event.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARNING",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"level": level, "message": message.0}));
    }
}
