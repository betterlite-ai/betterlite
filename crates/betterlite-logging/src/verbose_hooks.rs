use betterlite_redact::structured::{Payload, redact::redact_credentials_in_payload};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub enum HookInput {
    Main {
        payload: Payload,
        set_verbose: bool,
        #[serde(default)]
        logger_fails: bool,
        #[serde(default)]
        console_fails: bool,
    },
    Utils {
        payload: Payload,
        set_verbose: bool,
        log_level: String,
        logger_only: bool,
        #[serde(default)]
        logger_fails: bool,
        #[serde(default)]
        console_fails: bool,
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
                logger_fails,
                console_fails,
            } => Self::emit(
                payload,
                Some("DEBUG"),
                set_verbose,
                logger_fails,
                console_fails,
            ),
            HookInput::Utils {
                payload,
                set_verbose,
                log_level,
                logger_only,
                logger_fails,
                console_fails,
            } => {
                let level = match log_level.as_str() {
                    "DEBUG" | "INFO" | "ERROR" => Some(log_level.as_str()),
                    _ => None,
                };
                Self::emit(
                    payload,
                    level,
                    set_verbose && !logger_only,
                    logger_fails,
                    console_fails,
                )
            }
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
        logger_fails: bool,
        console_fails: bool,
    ) -> HookResult {
        let redacted = redact(payload);
        let mut result = HookResult::default();
        if logger_fails {
            return result;
        }
        if let Some(level_name) = level {
            result.logs.push(LogRecord {
                level: level_name.to_owned(),
                payload: redacted.clone(),
            });
        }
        if console_enabled && !console_fails {
            result.console.push(redacted);
        }
        result
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
