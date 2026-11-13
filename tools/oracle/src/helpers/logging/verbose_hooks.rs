use std::cell::Cell;
use std::sync::Arc;

use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use serde_json::json;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Main {
        payload: Payload,
        set_verbose: Payload,
        #[serde(default)]
        logger_fails: bool,
        #[serde(default)]
        console_fails: bool,
    },
    Utils {
        payload: Payload,
        set_verbose: Payload,
        #[serde(default = "default_log_level")]
        log_level: String,
        #[serde(default = "default_logger_only")]
        logger_only: Payload,
        #[serde(default)]
        logger_fails: bool,
        #[serde(default)]
        console_fails: bool,
    },
    Active {
        set_verbose: Payload,
        #[serde(default)]
        logger_debug_enabled: bool,
    },
}

fn default_log_level() -> String {
    "DEBUG".to_owned()
}

const fn default_logger_only() -> Payload {
    Payload::Bool(false)
}

struct FixtureLogger {
    fails: bool,
    debug_enabled: bool,
    records: Cell<Vec<betterlite_logging::verbose_hooks::LogRecord>>,
}

impl LoggerSink for FixtureLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.record("DEBUG", payload)
    }

    fn info(&self, payload: &Payload) -> bool {
        self.record("INFO", payload)
    }

    fn error(&self, payload: &Payload) -> bool {
        self.record("ERROR", payload)
    }

    fn debug_enabled(&self) -> bool {
        self.debug_enabled
    }
}

impl FixtureLogger {
    fn record(&self, level: &str, payload: &Payload) -> bool {
        if self.fails {
            return false;
        }
        let mut records = self.records.take();
        records.push(betterlite_logging::verbose_hooks::LogRecord {
            level: level.to_owned(),
            payload: payload.clone(),
        });
        self.records.set(records);
        true
    }
}

struct FixtureConsole {
    fails: bool,
}

impl ConsoleSink for FixtureConsole {
    fn print(&self, _payload: &Payload) -> bool {
        !self.fails
    }
}

fn redactor() -> Result<Arc<Redactor>, Box<crate::helpers::HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "RedactionError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })
    })?;
    Ok(Arc::new(redactor))
}

fn build_hooks(
    sink_fails: (bool, bool),
    logger_debug_enabled: bool,
    set_verbose: &Payload,
) -> Result<VerboseHooks<FixtureLogger, FixtureConsole>, Box<crate::helpers::HelperError>> {
    let (logger_fails, console_fails) = sink_fails;
    let redactor = redactor()?;
    Ok(VerboseHooks {
        logger: FixtureLogger {
            fails: logger_fails,
            debug_enabled: logger_debug_enabled,
            records: Cell::new(Vec::new()),
        },
        console: FixtureConsole {
            fails: console_fails,
        },
        set_verbose: set_verbose.clone(),
        redactor,
    })
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Main {
                payload,
                set_verbose,
                logger_fails,
                console_fails,
            } => run_logged(set_verbose, (*logger_fails, *console_fails), |hooks| {
                hooks.main_print_verbose(payload.clone())
            }),
            Input::Utils {
                payload,
                set_verbose,
                log_level,
                logger_only,
                logger_fails,
                console_fails,
            } => run_logged(set_verbose, (*logger_fails, *console_fails), |hooks| {
                hooks.utils_print_verbose(payload.clone(), log_level, logger_only)
            }),
            Input::Active {
                set_verbose,
                logger_debug_enabled,
            } => {
                let hooks = build_hooks((false, false), *logger_debug_enabled, set_verbose)?;
                Ok(json!({ "logs": [], "console": [], "active": hooks.is_active() }))
            }
        }
    }
}

fn run_logged<F>(
    set_verbose: &Payload,
    sink_fails: (bool, bool),
    operation: F,
) -> crate::helpers::HelperResult
where
    F: FnOnce(
        &VerboseHooks<FixtureLogger, FixtureConsole>,
    ) -> betterlite_logging::verbose_hooks::HookResult,
{
    let hooks = build_hooks(sink_fails, false, set_verbose)?;
    let mut result = operation(&hooks);
    result.logs = hooks.logger.records.take();
    Ok(json!(result))
}

#[path = "verbose_hooks/tests.rs"]
mod tests;
