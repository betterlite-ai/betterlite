use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// Collects the rendered message of every WARN event.
#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<String>>>);

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
    // The text before the first ": " is fixed; what follows names the transport failure.
    pub(super) fn split(&self) -> (Vec<String>, Vec<String>) {
        let messages = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        messages
            .iter()
            .map(|message| {
                let (fixed, detail) = message.split_once(": ").unwrap_or((message, ""));
                (fixed.to_owned(), detail.to_owned())
            })
            .unzip()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}
