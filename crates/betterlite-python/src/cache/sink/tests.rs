#![cfg(test)]

use betterlite_logging::verbose_hooks::{ConsoleSink as _, LoggerSink as _};
use betterlite_redact::structured::Payload;
use pyo3::prelude::*;

use super::{PythonConsole, PythonLog, debug_on, emit, logger, payload_text};

#[test]
fn a_string_message_logs_without_json_quotes() {
    assert_eq!(
        payload_text(&Payload::String("LiteLLM Cache: exception".to_owned())),
        "LiteLLM Cache: exception"
    );
}

#[test]
fn any_other_payload_keeps_its_json_text() {
    assert_eq!(payload_text(&Payload::Bool(true)), "true");
    assert_eq!(payload_text(&Payload::Null), "null");
}

#[test]
fn a_working_logger_and_console_answer_true() {
    Python::initialize();
    let payload = Payload::String("LiteLLM Cache: probe".to_owned());
    assert!(PythonLog.debug(&payload));
    assert!(PythonLog.info(&payload));
    assert!(PythonLog.error(&payload));
    assert!(PythonConsole.print(&payload));
    assert!(Python::attach(|py| logger(py).is_some()));
}

#[test]
fn a_logger_method_that_is_not_callable_answers_false() {
    Python::initialize();
    Python::attach(|py| {
        let log = logger(py).unwrap();
        log.setattr("debug", 1_u8).unwrap();
        log.setattr("info", 1_u8).unwrap();
        log.setattr("error", 1_u8).unwrap();
        let payload = Payload::String("LiteLLM Cache: probe".to_owned());
        assert!(!PythonLog.debug(&payload));
        assert!(!PythonLog.info(&payload));
        assert!(!PythonLog.error(&payload));
        assert!(!emit("debug", &payload));
        assert!(!emit("info", &payload));
        assert!(!emit("error", &payload));
    });
}

#[test]
fn a_print_that_is_not_callable_answers_false() {
    Python::initialize();
    Python::attach(|py| {
        let builtins = py.import("builtins").unwrap();
        let original_print = builtins.getattr("print").unwrap();
        builtins.setattr("print", 1_u8).unwrap();
        let printed = PythonConsole.print(&Payload::String("probe".to_owned()));
        builtins.setattr("print", original_print).unwrap();
        assert!(!printed);
    });
}

#[test]
fn debug_enabled_follows_the_logger_level() {
    Python::initialize();
    Python::attach(|py| {
        let log = logger(py).unwrap();
        log.call_method1("setLevel", (30_i32,)).unwrap();
        assert!(!PythonLog.debug_enabled());
        assert!(!debug_on(py));
        log.call_method1("setLevel", (10_i32,)).unwrap();
        assert!(PythonLog.debug_enabled());
        assert!(debug_on(py));
        log.call_method1("setLevel", (0_i32,)).unwrap();
    });
}
