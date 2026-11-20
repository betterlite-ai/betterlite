use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

// Every logged line the run wrote, read back after the wrapper is done.
#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Line(Option<String>);

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<S: Subscriber> Layer<S> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut line = Line::default();
        event.record(&mut line);
        if let Some(message) = line.0 {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(message);
        }
    }
}

impl Captured {
    // Only the two lines this wrapper writes, in order; B1119's event-failure line is compared by its own helper.
    pub(super) fn expected(&self, model: &str) -> Vec<String> {
        let wanted = [
            format!(
                "AnthropicResponsesStreamWrapper: upstream Responses stream for {model} failed"
            ),
            format!(
                "AnthropicResponsesStreamWrapper: upstream Responses stream for {model} ended without a terminal event"
            ),
        ];
        let lines = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut kept = Vec::new();
        for line in &*lines {
            if wanted.contains(line) {
                kept.push(line.clone());
            }
        }
        kept
    }
}

pub(super) fn install() -> (DefaultGuard, Captured) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    (set_default(subscriber), captured)
}
