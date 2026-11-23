use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_redact::structured::Payload;
use pyo3::prelude::*;

mod tests;

const LOGGER_NAME: &str = "LiteLLM";
const DEBUG_LEVEL: i32 = 10;

// LiteLLM's verbose_logger is logging.getLogger("LiteLLM"), so the cache's own lines land in the caller's log.
pub(crate) struct PythonLog;

impl LoggerSink for PythonLog {
    fn debug(&self, payload: &Payload) -> bool {
        emit("debug", payload)
    }

    fn info(&self, payload: &Payload) -> bool {
        emit("info", payload)
    }

    fn error(&self, payload: &Payload) -> bool {
        emit("error", payload)
    }

    fn debug_enabled(&self) -> bool {
        Python::attach(debug_on)
    }
}

// print_verbose prints when litellm.set_verbose is on; the cache passes that flag as set_verbose.
pub(crate) struct PythonConsole;

impl ConsoleSink for PythonConsole {
    fn print(&self, payload: &Payload) -> bool {
        Python::attach(|py| {
            py.import("builtins").is_ok_and(|builtins| {
                builtins
                    .call_method1("print", (payload_text(payload),))
                    .is_ok()
            })
        })
    }
}

fn emit(level: &str, payload: &Payload) -> bool {
    Python::attach(|py| {
        logger(py)
            .is_some_and(|logger| logger.call_method1(level, (payload_text(payload),)).is_ok())
    })
}

fn debug_on(py: Python<'_>) -> bool {
    logger(py).is_some_and(|logger| {
        logger
            .call_method1("isEnabledFor", (DEBUG_LEVEL,))
            .is_ok_and(|enabled| enabled.is_truthy().unwrap_or(false))
    })
}

fn logger<'py>(py: Python<'py>) -> Option<Bound<'py, PyAny>> {
    let logging = py.import("logging").ok()?;
    logging.call_method1("getLogger", (LOGGER_NAME,)).ok()
}

// A message the facade built is a string, so it logs unquoted; any other payload keeps its JSON text.
pub(crate) fn payload_text(payload: &Payload) -> String {
    match payload {
        Payload::String(text) => text.clone(),
        Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::Array(_)
        | Payload::Object(_) => payload.to_string(),
    }
}
