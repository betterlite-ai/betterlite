use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

#[derive(Clone, Default)]
pub(super) struct Logs {
    warnings: Arc<Mutex<Vec<String>>>,
    infos: Arc<Mutex<Vec<String>>>,
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
        if level != Level::WARN && level != Level::INFO {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        let target = if level == Level::WARN {
            &self.warnings
        } else {
            &self.infos
        };
        target
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

impl Logs {
    pub(super) fn take(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let warnings = {
            let mut guard = self.warnings.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };
        let infos = {
            let mut guard = self.infos.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };
        let mut fixed = Vec::with_capacity(warnings.len());
        let mut details = Vec::with_capacity(warnings.len());
        for warning in warnings {
            let (message, detail) = warning.split_once(": ").unwrap_or((&warning, ""));
            fixed.push(message.to_owned());
            details.push(detail.to_owned());
        }
        (fixed, details, infos)
    }
}
