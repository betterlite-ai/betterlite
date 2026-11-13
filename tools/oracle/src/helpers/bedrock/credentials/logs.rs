use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

pub(super) type LogEntry = (Level, String, Option<String>);

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<LogEntry>>>);

#[derive(Default)]
struct Fields {
    message: String,
    role: Option<String>,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "role" {
            self.role = Some(value.to_owned());
        }
    }
}

impl<S: Subscriber> Layer<S> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::ERROR {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push((
            level,
            fields.message.trim_matches('"').to_owned(),
            fields.role,
        ));
    }
}

impl Logs {
    pub(super) fn entries(&self) -> Vec<Value> {
        let entries = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        entries
            .into_iter()
            .filter_map(|(level, message, role)| log_json(level, &message, role.as_deref()))
            .collect()
    }
}

fn log_json(level: Level, message: &str, role: Option<&str>) -> Option<Value> {
    let code = if message.starts_with("AssumeRole failed for ") {
        "assume_role_failed_same_identity"
    } else if message.starts_with("AssumeRole AccessDenied for ") {
        "assume_role_denied_other_identity"
    } else if message.starts_with("Access denied when trying to assume role ") {
        "irsa_denied_trust_policy"
    } else {
        return None;
    };
    let name = if level == Level::ERROR {
        "error"
    } else {
        "warn"
    };
    Some(json!({"level": name, "code": code, "role": role}))
}
