use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

impl Logs {
    pub(super) fn take(&self, level: Level) -> Vec<String> {
        let records = &mut *self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut taken = Vec::new();
        let mut kept = Vec::new();
        for (recorded, message) in records.drain(..) {
            if recorded == level {
                taken.push(message);
            } else {
                kept.push((recorded, message));
            }
        }
        *records = kept;
        taken
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
        if level != Level::WARN && level != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, message.0));
    }
}

pub(super) fn capture_logs() -> (DefaultGuard, Logs) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    (set_default(subscriber), logs)
}
