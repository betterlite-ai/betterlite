use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::Subscriber;
use tracing::{Event, Level};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const PREFIX: &str = "rotate_sso_identity_assertions_master_key:";

// The Python side captures the same two levels off the "LiteLLM Proxy" logger and nothing else.
#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<(String, String)>>>);

impl Logs {
    pub(crate) fn entries(&self) -> Vec<(String, String)> {
        let entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        entries.clone()
    }

    fn record(&self, level: &'static str, message: String) {
        if !message.starts_with(PREFIX) {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level.to_owned(), message));
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Current> Layer<Current> for Logs
where
    Current: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Current>) {
        let level = match *event.metadata().level() {
            Level::WARN => "WARNING",
            Level::INFO => "INFO",
            Level::DEBUG | Level::ERROR | Level::TRACE => return,
        };
        let mut message = Message(String::new());
        event.record(&mut message);
        self.record(level, message.0);
    }
}
