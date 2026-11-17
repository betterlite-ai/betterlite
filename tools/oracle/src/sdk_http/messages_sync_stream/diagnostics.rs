use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::messages_sync_stream::driver::ADAPTER_LOG_PREFIX;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::sdk_http::result::SyncDiagnostic;

// Collects one run's adapter lines, from the module that run drives
#[derive(Clone)]
pub(crate) struct Diagnostics {
    pub(crate) source: &'static str,
    pub(crate) entries: Arc<Mutex<Vec<SyncDiagnostic>>>,
}

impl Diagnostics {
    // One step's lines, cleared so the next step reports only its own
    pub(crate) fn take(&self) -> Vec<SyncDiagnostic> {
        std::mem::take(&mut *self.entries.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[derive(Default)]
struct PrefixVisitor(Option<String>);

impl Visit for PrefixVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = adapter_prefix(&format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = adapter_prefix(value);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Diagnostics {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if !event.metadata().target().starts_with(self.source) {
            return;
        }
        let mut visitor = PrefixVisitor::default();
        event.record(&mut visitor);
        let Some(prefix) = visitor.0 else {
            return;
        };
        let level = match *event.metadata().level() {
            Level::ERROR => "error",
            Level::WARN => "warning",
            Level::INFO => "info",
            Level::DEBUG => "debug",
            Level::TRACE => "trace",
        };
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SyncDiagnostic {
                level: level.to_owned(),
                prefix,
            });
    }
}

// Only events that start with the adapter line are diagnostics; the rest of the failure text differs
fn adapter_prefix(message: &str) -> Option<String> {
    if !message.starts_with(ADAPTER_LOG_PREFIX) {
        return None;
    }
    Some(
        message
            .chars()
            .take(ADAPTER_LOG_PREFIX.chars().count())
            .collect(),
    )
}
