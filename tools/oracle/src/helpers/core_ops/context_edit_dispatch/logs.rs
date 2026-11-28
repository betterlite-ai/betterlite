use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

// the prefix the dispatcher's own debug line carries
const PREFIX: &str = "context_management polyfill:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct Diagnostic {
    pub(super) level: String,
    pub(super) message: String,
}

// the tracing lines one dispatch emitted, filtered to the dispatcher's own diagnostics
#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<Diagnostic>>>);

impl Logs {
    pub(super) fn diagnostics(&self) -> Vec<Diagnostic> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, level: &str, message: &str) {
        if !message.starts_with(PREFIX) {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Diagnostic {
                level: level.to_owned(),
                message: message.to_owned(),
            });
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
        let level = *event.metadata().level();
        let name = if level == Level::INFO {
            "info"
        } else if level == Level::WARN {
            "warning"
        } else if level == Level::DEBUG {
            "debug"
        } else {
            return;
        };
        let mut message = Message::default();
        event.record(&mut message);
        self.record(name, &message.0);
    }
}

pub(super) fn install() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    (logs, set_default(subscriber))
}
