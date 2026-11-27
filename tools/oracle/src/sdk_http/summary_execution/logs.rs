use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry;

use super::Diagnostic;

const MODULE_PREFIX: &str = "compact_20260112:";
const CALL_FAILED_PREFIX: &str = "compact_20260112: summary call failed:";
const CALL_FAILED: &str = "compact_20260112: summary call failed";
const COUNT_FAILED_PREFIX: &str =
    "compact_20260112: token_counter failed; assuming under threshold:";
const COUNT_FAILED: &str = "compact_20260112: token_counter failed; assuming under threshold";

#[derive(Clone, Default)]
pub(super) struct Diagnostics(Arc<Mutex<Vec<Diagnostic>>>);

impl Diagnostics {
    pub(super) fn lines(&self) -> Vec<Diagnostic> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// the exception text of the call-failure and counter-failure warnings belongs to one runtime only
fn normalize(message: String) -> String {
    if message.starts_with(CALL_FAILED_PREFIX) {
        return String::from(CALL_FAILED);
    }
    if message.starts_with(COUNT_FAILED_PREFIX) {
        return String::from(COUNT_FAILED);
    }
    message
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

impl<Inner: Subscriber> Layer<Inner> for Diagnostics {
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
        if !message.0.starts_with(MODULE_PREFIX) {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Diagnostic {
                level: name.to_owned(),
                message: normalize(message.0),
            });
    }
}

pub(super) fn capture() -> (Diagnostics, DefaultGuard) {
    let diagnostics = Diagnostics::default();
    let subscriber = registry().with(diagnostics.clone());
    let guard = set_default(subscriber);
    (diagnostics, guard)
}
