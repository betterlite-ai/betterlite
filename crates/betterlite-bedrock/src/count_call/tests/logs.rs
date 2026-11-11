use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Debug,
    Warn,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) kind: Option<Kind>,
    pub(crate) subject: Option<String>,
    pub(crate) value: Option<String>,
    pub(crate) status: Option<u64>,
    pub(crate) detail: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct Captured {
    entries: Arc<Mutex<Vec<Entry>>>,
}

impl Captured {
    pub(crate) fn entries(&self) -> Vec<Entry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn of(&self, kind: Kind) -> Vec<Entry> {
        self.entries()
            .into_iter()
            .filter(|entry| entry.kind == Some(kind))
            .collect()
    }

    pub(crate) fn subjects(&self, kind: Kind) -> Vec<String> {
        self.of(kind)
            .into_iter()
            .filter_map(|entry| entry.subject)
            .collect()
    }

    pub(crate) fn text(&self) -> String {
        let mut text = String::new();
        for entry in self.entries() {
            for field in [entry.subject, entry.value, entry.detail]
                .into_iter()
                .flatten()
            {
                text.push_str(&field);
                text.push('\n');
            }
        }
        text
    }

    fn push(&self, entry: Entry) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

struct Recorded {
    entry: Entry,
}

impl Recorded {
    fn assign(&mut self, name: &str, text: String) {
        match name {
            "subject" => self.entry.subject = Some(text),
            "value" => self.entry.value = Some(text),
            "detail" => self.entry.detail = Some(text),
            _ => {}
        }
    }
}

impl Visit for Recorded {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.assign(field.name(), value.to_owned());
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

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.assign(field.name(), format!("{value:?}"));
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut recorded = Recorded {
            entry: Entry {
                kind: Some(match *event.metadata().level() {
                    Level::ERROR => Kind::Error,
                    Level::WARN => Kind::Warn,
                    Level::INFO | Level::DEBUG | Level::TRACE => Kind::Debug,
                }),
                subject: None,
                value: None,
                status: None,
                detail: None,
            },
        };
        event.record(&mut recorded);
        if recorded.entry.subject.is_some() {
            self.push(recorded.entry);
        }
    }
}

pub(crate) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let guard = set_default(subscriber(captured.clone()));
    (captured, guard)
}

fn subscriber(captured: Captured) -> impl Subscriber + Send + Sync {
    tracing_subscriber::registry().with(captured)
}
