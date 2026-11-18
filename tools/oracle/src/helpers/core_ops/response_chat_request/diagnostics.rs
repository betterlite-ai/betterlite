use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use std::mem;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// the three builder messages the helper compares, so other chat provider logs stay out of the answer
const WATCHED: [&str; 3] = [
    "Chat provider: Stream parameter:",
    "Chat provider: Warning ignoring previous response ID:",
    "Chat provider: Final request model=",
];

#[derive(Clone, Default)]
pub(super) struct Diagnostics(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Captured(String);

impl Visit for Captured {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Diagnostics {
    pub(super) fn drain(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Diagnostics {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut captured = Captured::default();
        event.record(&mut captured);
        if !WATCHED.iter().any(|prefix| captured.0.starts_with(prefix)) {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(captured.0);
    }
}
