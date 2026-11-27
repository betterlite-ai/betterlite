use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// the two levels _check_summary_model_access logs, in the words the Python adapter records
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct DiagnosticRecord {
    pub level: &'static str,
    pub message: String,
}

#[derive(Clone, Default)]
pub(super) struct Diagnostics(Arc<Mutex<Vec<DiagnosticRecord>>>);

impl Diagnostics {
    pub(super) fn lines(&self) -> Vec<DiagnosticRecord> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

impl<Inner: Subscriber> Layer<Inner> for Diagnostics {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        let name = if level == Level::WARN {
            "warning"
        } else if level == Level::DEBUG {
            "debug"
        } else {
            return;
        };
        let mut message = Message::default();
        event.record(&mut message);
        if !message.0.starts_with("compact_20260112:") {
            return;
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(DiagnosticRecord {
                level: name,
                message: message.0,
            });
    }
}
