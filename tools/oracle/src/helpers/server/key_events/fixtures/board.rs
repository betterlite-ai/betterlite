use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// one board holds what every collaborator saw, in the order the hook reached it
#[derive(Default)]
pub(crate) struct Board {
    trace: Mutex<Vec<Value>>,
}

impl Board {
    pub(crate) fn record(&self, entry: Value) {
        self.trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(crate) fn entries(&self) -> Vec<Value> {
        self.trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Levels {
    pub(crate) board: Arc<Board>,
}

impl<S: Subscriber> Layer<S> for Levels {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        if *metadata.level() != Level::WARN
            || !metadata
                .target()
                .starts_with("betterlite_server::key_event")
        {
            return;
        }
        // LiteLLM's warning text names its private helper and its arguments, so the trace orders
        // warnings by level alone, as the B898 trace does
        self.board.record(json!(["warn"]));
    }
}
