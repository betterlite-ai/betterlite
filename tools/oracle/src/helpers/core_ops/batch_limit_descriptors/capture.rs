use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

#[derive(Clone)]
pub(super) struct Capture(pub(super) Arc<Mutex<Vec<Value>>>);

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        // the Python logger is set to WARNING, so only these two levels reach its collector
        if !matches!(level, Level::ERROR | Level::WARN) {
            return;
        }
        let name = match level {
            Level::ERROR => "error",
            Level::WARN => "warning",
            Level::INFO => "info",
            Level::DEBUG => "debug",
            Level::TRACE => "trace",
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"event": "warning", "level": name}));
    }
}
