use std::sync::Arc;

use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::{Payload, REDACTED, redact::redact_credentials_in_payload};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogRecord {
    pub level: String,
    pub payload: Payload,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct HookResult {
    pub logs: Vec<LogRecord>,
    pub console: Vec<Payload>,
    pub active: bool,
}

pub trait LoggerSink {
    fn debug(&self, payload: &Payload) -> bool;
    fn info(&self, payload: &Payload) -> bool;
    fn error(&self, payload: &Payload) -> bool;
    fn debug_enabled(&self) -> bool;
}

pub trait ConsoleSink {
    fn print(&self, payload: &Payload) -> bool;
}

pub struct VerboseHooks<L, C> {
    pub logger: L,
    pub console: C,
    pub set_verbose: Payload,
    pub redactor: Arc<Redactor>,
}

impl<L: LoggerSink, C: ConsoleSink> VerboseHooks<L, C> {
    // Both sinks run inside `print_verbose`'s single try, so a logger failure skips the console.
    #[must_use]
    pub fn main_print_verbose(&self, payload: Payload) -> HookResult {
        let redacted = redact(payload, &self.redactor);
        let mut result = HookResult::default();
        if !self.logger.debug(&redacted) {
            return result;
        }
        result.logs.push(LogRecord {
            level: "DEBUG".to_owned(),
            payload: redacted.clone(),
        });
        if main_guard_allows_console(&self.set_verbose) && self.console.print(&redacted) {
            result.console.push(redacted);
        }
        result
    }

    // An unknown log_level logs nothing but still allows the console print.
    #[must_use]
    pub fn utils_print_verbose(
        &self,
        payload: Payload,
        log_level: &str,
        logger_only: &Payload,
    ) -> HookResult {
        let redacted = redact(payload, &self.redactor);
        let mut result = HookResult::default();
        let logged = match log_level {
            "DEBUG" => Some(self.logger.debug(&redacted)),
            "INFO" => Some(self.logger.info(&redacted)),
            "ERROR" => Some(self.logger.error(&redacted)),
            _ => None,
        };
        if logged == Some(false) {
            return result;
        }
        if logged == Some(true) {
            result.logs.push(LogRecord {
                level: log_level.to_owned(),
                payload: redacted.clone(),
            });
        }
        if is_true(&self.set_verbose) && is_false(logger_only) && self.console.print(&redacted) {
            result.console.push(redacted);
        }
        result
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        if is_true(&self.set_verbose) {
            return true;
        }
        self.logger.debug_enabled()
    }
}

fn main_guard_allows_console(value: &Payload) -> bool {
    match value {
        Payload::Null => false,
        Payload::Bool(flag) => *flag,
        Payload::Number(number) => number.as_f64().is_none_or(|parsed| parsed != 0.0_f64),
        Payload::String(text) => !text.is_empty(),
        Payload::Array(items) => !items.is_empty(),
        Payload::Object(fields) => !fields.is_empty(),
    }
}

const fn is_true(value: &Payload) -> bool {
    if let Payload::Bool(flag) = value {
        *flag
    } else {
        false
    }
}

const fn is_false(value: &Payload) -> bool {
    if let Payload::Bool(flag) = value {
        !*flag
    } else {
        false
    }
}

// Strings and array items carry no key for B187's object masking, so content-scan them too.
fn redact(payload: Payload, redactor: &Redactor) -> Payload {
    match payload {
        Payload::Object(object) => {
            let masked = redact_credentials_in_payload(&object);
            let Payload::Object(fields) = masked else {
                return masked;
            };
            Payload::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key.clone(), redact_field(value, &key, redactor)))
                    .collect(),
            )
        }
        Payload::Array(items) => Payload::Array(
            items
                .into_iter()
                .map(|item| redact(item, redactor))
                .collect(),
        ),
        Payload::String(text) => Payload::String(redactor.redact_string(&text)),
        Payload::Null | Payload::Bool(_) | Payload::Number(_) => payload,
    }
}

fn redact_field(value: Payload, key: &str, redactor: &Redactor) -> Payload {
    if matches!(key, "message" | "content" | "prompt" | "text") {
        return Payload::String(REDACTED.to_owned());
    }
    match value {
        Payload::String(text) => Payload::String(redactor.redact_string(&text)),
        Payload::Null => Payload::Null,
        Payload::Bool(flag) => Payload::Bool(flag),
        Payload::Number(number) => Payload::Number(number),
        Payload::Array(items) => redact(Payload::Array(items), redactor),
        Payload::Object(fields) => redact(Payload::Object(fields), redactor),
    }
}

mod tests;
