use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

const RECOVERY: &str = "Recovered empty Responses API output from raw SSE";
const RECOVERY_LABEL: &str = "recovered_from_sse";

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    pub(super) fn labels(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|message| message.contains(RECOVERY))
            .map(|_| RECOVERY_LABEL.to_owned())
            .collect()
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

impl<Inner: Subscriber> Layer<Inner> for Warnings {
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
