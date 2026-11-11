use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Debug, Clone, Default)]
pub(super) struct Entry {
    pub(super) level: Option<Level>,
    pub(super) subject: Option<String>,
    pub(super) value: Option<String>,
    pub(super) status: Option<u64>,
}

#[derive(Clone, Default)]
pub(super) struct Captured {
    entries: Arc<Mutex<Vec<Entry>>>,
}

impl Captured {
    pub(super) fn entries(&self, level: Level) -> Vec<Entry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|entry| entry.level == Some(level))
            .cloned()
            .collect()
    }

    fn push(&self, entry: Entry) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let guard = set_default(tracing_subscriber::registry().with(captured.clone()));
    (captured, guard)
}

struct Recorded {
    entry: Entry,
}

impl Visit for Recorded {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "subject" => self.entry.subject = Some(value.to_owned()),
            "value" => self.entry.value = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "status" {
            self.entry.status = Some(value);
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        if field.name() == "status" {
            self.entry.status = u64::try_from(value).ok();
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut recorded = Recorded {
            entry: Entry {
                level: Some(*event.metadata().level()),
                subject: None,
                value: None,
                status: None,
            },
        };
        event.record(&mut recorded);
        if recorded.entry.subject.is_some() {
            self.push(recorded.entry);
        }
    }
}
