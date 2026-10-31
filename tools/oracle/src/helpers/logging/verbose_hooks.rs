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
}

impl LoggerSink for FixtureLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        !self.fails
    }

    fn info(&self, _payload: &Payload) -> bool {
        !self.fails
    }

    fn error(&self, _payload: &Payload) -> bool {
        !self.fails
    }

    fn debug_enabled(&self) -> bool {
        self.debug_enabled
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
    logger: FixtureLogger,
    console: FixtureConsole,
    set_verbose: &Payload,
) -> Result<VerboseHooks<FixtureLogger, FixtureConsole>, Box<crate::helpers::HelperError>> {
    let redactor = redactor()?;
    Ok(VerboseHooks {
        logger,
        console,
        set_verbose: set_verbose.clone(),
        redactor,
    })
}

struct MainCall<'call> {
    payload: &'call Payload,
    set_verbose: &'call Payload,
    logger_fails: bool,
    console_fails: bool,
}

fn run_main(call: &MainCall<'_>) -> crate::helpers::HelperResult {
    let hooks = build_hooks(
        FixtureLogger {
            fails: call.logger_fails,
            debug_enabled: false,
        },
        FixtureConsole {
            fails: call.console_fails,
        },
        call.set_verbose,
    )?;
    Ok(json!(hooks.main_print_verbose(call.payload.clone())))
}

struct UtilsCall<'call> {
    payload: &'call Payload,
    set_verbose: &'call Payload,
    log_level: &'call str,
    logger_only: &'call Payload,
    logger_fails: bool,
    console_fails: bool,
}

fn run_utils(call: &UtilsCall<'_>) -> crate::helpers::HelperResult {
    let hooks = build_hooks(
        FixtureLogger {
            fails: call.logger_fails,
            debug_enabled: false,
        },
        FixtureConsole {
            fails: call.console_fails,
        },
        call.set_verbose,
    )?;
    Ok(json!(hooks.utils_print_verbose(
        call.payload.clone(),
        call.log_level,
        call.logger_only
    )))
}

fn run_active(set_verbose: &Payload, logger_debug_enabled: bool) -> crate::helpers::HelperResult {
    let hooks = build_hooks(
        FixtureLogger {
            fails: false,
            debug_enabled: logger_debug_enabled,
        },
        FixtureConsole { fails: false },
        set_verbose,
    )?;
    Ok(json!({ "logs": [], "console": [], "active": hooks.is_active() }))
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
            } => run_main(&MainCall {
                payload,
                set_verbose,
                logger_fails: *logger_fails,
                console_fails: *console_fails,
            }),
            Input::Utils {
                payload,
                set_verbose,
                log_level,
                logger_only,
                logger_fails,
                console_fails,
            } => run_utils(&UtilsCall {
                payload,
                set_verbose,
                log_level,
                logger_only,
                logger_fails: *logger_fails,
                console_fails: *console_fails,
            }),
            Input::Active {
                set_verbose,
                logger_debug_enabled,
            } => run_active(set_verbose, *logger_debug_enabled),
        }
    }
}

#[path = "verbose_hooks/tests.rs"]
mod tests;
