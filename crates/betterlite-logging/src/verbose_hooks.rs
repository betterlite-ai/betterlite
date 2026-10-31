use betterlite_redact::structured::{Payload, redact::redact_credentials_in_payload};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct SinkFailures {
    #[serde(default)]
    pub logger: bool,
    #[serde(default)]
    pub console: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub enum HookInput {
    Main {
        payload: Payload,
        set_verbose: bool,
        #[serde(default)]
        sinks: SinkFailures,
    },
    Utils {
        payload: Payload,
        set_verbose: bool,
        log_level: String,
        logger_only: bool,
        #[serde(default)]
        sinks: SinkFailures,
    },
    Active {
        set_verbose: bool,
        logger_debug_enabled: bool,
    },
}

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

#[derive(Debug, Clone, Copy, Default)]
pub struct VerboseHooks;

impl VerboseHooks {
    #[must_use]
    pub fn run(&self, input: HookInput) -> HookResult {
        match input {
            HookInput::Main {
                payload,
                set_verbose,
                sinks,
            } => Self::emit(payload, Some("DEBUG"), set_verbose, sinks),
            HookInput::Utils {
                payload,
                set_verbose,
                log_level,
                logger_only,
                sinks,
            } => Self::emit(
                payload,
                known_level(&log_level),
                set_verbose && !logger_only,
                sinks,
            ),
            HookInput::Active {
                set_verbose,
                logger_debug_enabled,
            } => HookResult {
                logs: Vec::new(),
                console: Vec::new(),
                active: set_verbose || logger_debug_enabled,
            },
        }
    }

    fn emit(
        payload: Payload,
        level: Option<&str>,
        console_enabled: bool,
        sinks: SinkFailures,
    ) -> HookResult {
        let redacted = redact(payload);
        let mut result = HookResult::default();
        if let Some(level_name) = level {
            if sinks.logger {
                return result;
            }
            result.logs.push(LogRecord {
                level: level_name.to_owned(),
                payload: redacted.clone(),
            });
        }
        if console_enabled && !sinks.console {
            result.console.push(redacted);
        }
        result
    }
}

fn known_level(log_level: &str) -> Option<&str> {
    match log_level {
        "DEBUG" | "INFO" | "ERROR" => Some(log_level),
        _ => None,
    }
}

fn redact(payload: Payload) -> Payload {
    match payload {
        Payload::Object(object) => redact_credentials_in_payload(&object),
        Payload::Null => Payload::Null,
        Payload::Bool(value) => Payload::Bool(value),
        Payload::Number(value) => Payload::Number(value),
        Payload::String(value) => Payload::String(value),
        Payload::Array(value) => Payload::Array(value),
    }
}

mod tests;
