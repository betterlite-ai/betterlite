use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, Subscriber, set_default};
use tracing::{Event, Level};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::arguments::json::{Json, Object};
use crate::cache_accounting::dialect::ModelFacts;
use crate::cache_accounting::points::{CacheControlInjectionPoint, MessageInjectionPoint};
use crate::message_content::Message;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn object(text: &str) -> Object {
    json(text).as_object().cloned().unwrap()
}

pub(super) fn messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn points(text: &str) -> Vec<CacheControlInjectionPoint> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn message_points(text: &str) -> Vec<MessageInjectionPoint> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn point(text: &str) -> MessageInjectionPoint {
    serde_json::from_str(text).unwrap()
}

pub(super) fn wire<T>(value: &T) -> Json
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).unwrap();
    serde_json::from_value(encoded).unwrap()
}

pub(super) struct Facts {
    flags: BTreeMap<String, bool>,
    provider: Option<String>,
}

impl Facts {
    pub(super) fn none() -> Self {
        Self {
            flags: BTreeMap::new(),
            provider: None,
        }
    }
}

impl ModelFacts for Facts {
    fn cache_breakpoint_flag(&self, model: &str) -> Option<bool> {
        self.flags.get(model).copied()
    }

    fn provider(&self, _model: &str) -> Option<String> {
        self.provider.clone()
    }
}

#[derive(Clone, Default)]
struct CapturedEvents(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct CapturedFields(Vec<String>);

impl Visit for CapturedFields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0.push(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push(value.to_owned());
        }
    }
}

impl<S: Subscriber> Layer<S> for CapturedEvents {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut fields = CapturedFields::default();
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.extend(fields.0);
        }
    }
}

pub(super) fn capture_warnings() -> (Arc<Mutex<Vec<String>>>, DefaultGuard) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturedEvents(Arc::clone(&events)));
    (events, set_default(subscriber))
}
