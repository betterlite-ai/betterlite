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
            } => Self::main(MainOptions {
                payload,
                set_verbose,
                logger_failure: failure(logger_fails),
                console_failure: failure(console_fails),
            }),
            HookInput::Utils {
                payload,
                set_verbose,
                log_level,
                logger_only,
                logger_fails,
                console_fails,
            } => Self::utils(UtilsOptions {
                payload,
                set_verbose,
                log_level,
                logger_only,
                logger_failure: failure(logger_fails),
                console_failure: failure(console_fails),
            }),
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

    fn main(options: MainOptions) -> HookResult {
        Self::emit(EmitOptions {
            payload: options.payload,
            level: Some("DEBUG"),
            console_enabled: options.set_verbose,
            logger_failure: options.logger_failure,
            console_failure: options.console_failure,
        })
    }

    fn utils(options: UtilsOptions) -> HookResult {
        let level = match options.log_level.as_str() {
            "DEBUG" | "INFO" | "ERROR" => Some(options.log_level.as_str()),
            _ => None,
        };
        Self::emit(EmitOptions {
            payload: options.payload,
            level,
            console_enabled: options.set_verbose && !options.logger_only,
            logger_failure: options.logger_failure,
            console_failure: options.console_failure,
        })
    }

    fn emit(options: EmitOptions<'_>) -> HookResult {
        let redacted = redact(options.payload);
        let mut result = HookResult::default();
        if matches!(options.logger_failure, SinkFailure::Failed) {
            return result;
        }
        if let Some(level_name) = options.level {
            result.logs.push(LogRecord {
                level: level_name.to_owned(),
                payload: redacted.clone(),
            });
        }
        if options.console_enabled && matches!(options.console_failure, SinkFailure::Succeeded) {
            result.console.push(redacted);
        }
        result
    }
}

struct EmitOptions<'payload> {
    payload: Payload,
    level: Option<&'payload str>,
    console_enabled: bool,
    logger_failure: SinkFailure,
    console_failure: SinkFailure,
}

struct UtilsOptions {
    payload: Payload,
    set_verbose: bool,
    log_level: String,
    logger_only: bool,
    logger_failure: SinkFailure,
    console_failure: SinkFailure,
}

struct MainOptions {
    payload: Payload,
    set_verbose: bool,
    logger_failure: SinkFailure,
    console_failure: SinkFailure,
}

#[derive(Clone, Copy)]
enum SinkFailure {
    Succeeded,
    Failed,
}

const fn failure(failed: bool) -> SinkFailure {
    if failed {
        SinkFailure::Failed
    } else {
        SinkFailure::Succeeded
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
