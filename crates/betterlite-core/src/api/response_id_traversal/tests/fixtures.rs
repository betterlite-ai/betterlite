use std::fmt::Debug;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use indexmap::IndexMap;
use serde_json::Number;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use betterlite_models::decisions::Json;

use crate::api::response_id_traversal::record::{AttributeObject, DictRecord, Record};

pub(super) fn text(value: &str) -> Record {
    Record::Scalar(Json::Text(value.to_owned()))
}

pub(super) fn number(value: i64) -> Record {
    Record::Scalar(Json::Number(Number::from(value)))
}

pub(super) fn null() -> Record {
    Record::Scalar(Json::Null)
}

pub(super) fn dict(entries: Vec<(&str, Record)>) -> Record {
    Record::Dict(DictRecord {
        values: fields(entries),
    })
}

pub(super) fn text_field(record: &Record, name: &str) -> Option<String> {
    record
        .field(name)
        .and_then(Record::as_text)
        .map(str::to_owned)
}

pub(super) fn fields(entries: Vec<(&str, Record)>) -> IndexMap<String, Record> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

pub(super) fn object(entries: Vec<(&str, Record)>, readonly: &[&str]) -> Record {
    Record::Object {
        object: AttributeObject {
            fields: fields(entries),
            readonly: readonly.iter().map(|name| (*name).to_owned()).collect(),
        },
    }
}

pub(super) fn list(items: Vec<Record>) -> Record {
    Record::List(items)
}

pub(super) fn output(items: Vec<Record>) -> Record {
    dict(vec![("output", list(items))])
}

pub(super) fn nested_call(container_id: &str, as_object: bool) -> Record {
    if as_object {
        object(vec![("container_id", text(container_id))], &[])
    } else {
        dict(vec![("container_id", text(container_id))])
    }
}

pub(super) fn message_with_annotation(container_id: &str, as_object: bool) -> Record {
    let annotation = nested_call(container_id, as_object);
    let part = if as_object {
        object(vec![("annotations", list(vec![annotation]))], &[])
    } else {
        dict(vec![("annotations", list(vec![annotation]))])
    };
    if as_object {
        object(
            vec![("type", text("message")), ("content", list(vec![part]))],
            &[],
        )
    } else {
        dict(vec![
            ("type", text("message")),
            ("content", list(vec![part])),
        ])
    }
}

pub(super) fn container_of(record: &Record) -> Option<String> {
    text_field(record, "container_id")
}

pub(super) fn nested_container_of(record: &Record) -> Option<String> {
    text_field(record.field("code_interpreter_call")?, "container_id")
}

pub(super) fn annotation_container_of(record: &Record) -> Option<String> {
    let Record::List(content) = record.field("content")? else {
        return None;
    };
    let part = content.first()?;
    let Record::List(annotations) = part.field("annotations")? else {
        return None;
    };
    text_field(annotations.first()?, "container_id")
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    pub(super) fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture<T, F>(call: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs.take())
}
