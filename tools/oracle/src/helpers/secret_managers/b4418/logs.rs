use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const EVENTS: [(&str, &str); 11] = [
    ("Authenticating with CyberArk Conjur", "authenticating"),
    ("Successfully authenticated", "authenticated"),
    ("SSL verification is disabled", "ssl_verification_disabled"),
    ("Created policy entry", "policy_created"),
    (
        "policy was rejected as unprocessable",
        "policy_unprocessable",
    ),
    ("Could not ensure variable exists", "policy_refused"),
    ("Error ensuring variable exists", "policy_failed"),
    ("not found in CyberArk Conjur", "secret_not_found"),
    ("Error reading secret from CyberArk Conjur", "read_failed"),
    ("Error writing secret to CyberArk Conjur", "write_failed"),
    (
        "does not support direct secret deletion",
        "delete_not_supported",
    ),
];

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<Value>>>);

impl Logs {
    pub(super) fn take(&self) -> Vec<Value> {
        let mut taken = std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner));
        // LiteLLM's base rotation logs an AWS-worded error that betterlite does not
        taken.retain(|entry| entry["event"] != "unrecognized");
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
        let mut message = Message::default();
        event.record(&mut message);
        let name = EVENTS
            .iter()
            .find(|(fragment, _)| message.0.contains(fragment))
            .map_or("unrecognized", |(_, name)| name);
        let level = if *event.metadata().level() == Level::WARN {
            "warning".to_owned()
        } else {
            event.metadata().level().as_str().to_lowercase()
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"level": level, "event": name}));
    }
}
