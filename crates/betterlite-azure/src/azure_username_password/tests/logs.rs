use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FieldValue {
    Text(String),
    Bool(bool),
}

#[derive(Debug, Clone)]
pub(super) struct CapturedEvent {
    pub(super) level: Level,
    pub(super) message: String,
    pub(super) fields: BTreeMap<String, FieldValue>,
}

#[derive(Clone, Default)]
pub(super) struct Capture(Arc<Mutex<Vec<CapturedEvent>>>);

impl Capture {
    pub(super) fn events(&self) -> Vec<CapturedEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Visitor {
    message: String,
    fields: BTreeMap<String, FieldValue>,
}

impl Visitor {
    fn record(&mut self, name: &str, text: String) {
        if name == "message" {
            self.message = text;
        } else {
            self.fields.insert(name.to_owned(), FieldValue::Text(text));
        }
    }
}

impl Visit for Visitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.record(field.name(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field.name(), value.to_owned());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), FieldValue::Bool(value));
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CapturedEvent {
                level: *event.metadata().level(),
                message: visitor.message,
                fields: visitor.fields,
            });
    }
}
