#![cfg(test)]

use betterlite_redact::structured::Payload;
use rstest::rstest;
use serde_json::json;

use super::{HookInput, SinkFailures, VerboseHooks};

fn credential_payload() -> Payload {
    serde_json::from_str(r#"{"api_key": "secret", "message": "body"}"#).expect("valid json fixture")
}

#[rstest]
#[case::main_verbose_guard(HookInput::Main { payload: json!("message"), set_verbose: true, sinks: SinkFailures::default() }, 1, 1, false)]
#[case::utils_verbose_levels(HookInput::Utils { payload: json!("message"), set_verbose: true, log_level: "INFO".to_owned(), logger_only: false, sinks: SinkFailures::default() }, 1, 1, false)]
#[case::verbose_active(HookInput::Active { set_verbose: false, logger_debug_enabled: true }, 0, 0, true)]
#[case::redacted_verbose_payload(HookInput::Main { payload: credential_payload(), set_verbose: true, sinks: SinkFailures::default() }, 1, 1, false)]
fn verbose_hooks_cases(
    #[case] input: HookInput,
    #[case] logs: usize,
    #[case] console: usize,
    #[case] active: bool,
) {
    let result = VerboseHooks.run(input);
    assert_eq!(result.logs.len(), logs);
    assert_eq!(result.console.len(), console);
    assert_eq!(result.active, active);
}

#[test]
fn redacts_credentials_before_any_sink_sees_them() {
    let result = VerboseHooks.run(HookInput::Main {
        payload: credential_payload(),
        set_verbose: true,
        sinks: SinkFailures::default(),
    });
    assert_eq!(result.logs[0].payload["api_key"], "REDACTED");
    assert_eq!(result.console[0]["message"], "body");
}

#[test]
fn logger_failure_suppresses_both_sinks() {
    let main_failed = VerboseHooks.run(HookInput::Main {
        payload: json!("body"),
        set_verbose: true,
        sinks: SinkFailures {
            logger: true,
            console: false,
        },
    });
    assert!(main_failed.logs.is_empty());
    assert!(main_failed.console.is_empty());

    let utils_failed = VerboseHooks.run(HookInput::Utils {
        payload: json!("body"),
        set_verbose: true,
        log_level: "DEBUG".to_owned(),
        logger_only: false,
        sinks: SinkFailures {
            logger: true,
            console: false,
        },
    });
    assert!(utils_failed.logs.is_empty());
    assert!(utils_failed.console.is_empty());
}

#[test]
fn logger_only_suppresses_the_console_even_when_verbose_is_set() {
    let result = VerboseHooks.run(HookInput::Utils {
        payload: json!("body"),
        set_verbose: true,
        log_level: "DEBUG".to_owned(),
        logger_only: true,
        sinks: SinkFailures::default(),
    });
    assert_eq!(result.logs.len(), 1);
    assert!(result.console.is_empty());
}

#[test]
fn unknown_log_level_skips_the_logger_but_not_the_console() {
    let result = VerboseHooks.run(HookInput::Utils {
        payload: json!("body"),
        set_verbose: true,
        log_level: "TRACE".to_owned(),
        logger_only: false,
        sinks: SinkFailures {
            logger: true,
            console: false,
        },
    });
    assert!(result.logs.is_empty());
    assert_eq!(result.console.len(), 1);
}

#[test]
fn console_failure_does_not_drop_the_logger_record() {
    let result = VerboseHooks.run(HookInput::Main {
        payload: json!("body"),
        set_verbose: true,
        sinks: SinkFailures {
            logger: false,
            console: true,
        },
    });
    assert_eq!(result.logs.len(), 1);
    assert!(result.console.is_empty());
}
