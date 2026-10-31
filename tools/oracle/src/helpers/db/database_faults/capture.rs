use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

#[derive(Clone, Serialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub(super) enum Effect {
    Read,
    Reconnect {
        reason: String,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    },
    Warning {
        fields: BTreeMap<String, String>,
    },
}

#[derive(Clone)]
pub(super) struct Capture(pub Arc<Mutex<Vec<Effect>>>);
struct Fields(BTreeMap<String, String>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() != "message" {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() != "message" {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }
}
impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if *event.metadata().level() == tracing::Level::WARN {
            let mut fields = Fields(BTreeMap::new());
            event.record(&mut fields);
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Effect::Warning { fields: fields.0 });
        }
    }
}
