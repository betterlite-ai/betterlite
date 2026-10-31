#![cfg(test)]

use std::sync::Arc;

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use rstest::rstest;
use serde_json::json;

use super::{ConsoleSink, LoggerSink, VerboseHooks};

#[derive(Default)]
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

#[derive(Default)]
struct FixtureConsole {
    fails: bool,
}

impl ConsoleSink for FixtureConsole {
    fn print(&self, _payload: &Payload) -> bool {
        !self.fails
    }
}

fn redactor() -> Arc<Redactor> {
    Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redaction patterns compile"))
}

fn credential_payload() -> Payload {
    serde_json::from_str(r#"{"api_key": "secret", "message": "body"}"#).expect("valid json fixture")
}

fn hooks(
    logger: FixtureLogger,
    console: FixtureConsole,
    set_verbose: Payload,
) -> VerboseHooks<FixtureLogger, FixtureConsole> {
    VerboseHooks {
        logger,
        console,
        set_verbose,
        redactor: redactor(),
    }
}

fn healthy_hooks(set_verbose: Payload) -> VerboseHooks<FixtureLogger, FixtureConsole> {
    hooks(
        FixtureLogger::default(),
        FixtureConsole::default(),
        set_verbose,
    )
}

enum Scenario {
    Main {
        payload: Payload,
        set_verbose: Payload,
    },
    Utils {
        payload: Payload,
        set_verbose: Payload,
        log_level: &'static str,
        logger_only: Payload,
    },
    Active {
        set_verbose: Payload,
        logger_debug_enabled: bool,
    },
}

struct Expected {
    logs: usize,
    console: usize,
    active: bool,
}

#[rstest]
#[case::main_verbose_guard(
    Scenario::Main { payload: json!("message"), set_verbose: json!(true) },
    Expected { logs: 1, console: 1, active: false }
)]
#[case::utils_verbose_levels(
    Scenario::Utils {
        payload: json!("message"),
        set_verbose: json!(true),
        log_level: "INFO",
        logger_only: json!(false),
    },
    Expected { logs: 1, console: 1, active: false }
)]
#[case::verbose_active(
    Scenario::Active { set_verbose: json!(false), logger_debug_enabled: true },
    Expected { logs: 0, console: 0, active: true }
)]
#[case::redacted_verbose_payload(
    Scenario::Main { payload: credential_payload(), set_verbose: json!(true) },
    Expected { logs: 1, console: 1, active: false }
)]
fn verbose_hooks_cases(#[case] scenario: Scenario, #[case] expected: Expected) {
    let result = match scenario {
        Scenario::Main {
            payload,
            set_verbose,
        } => healthy_hooks(set_verbose).main_print_verbose(payload),
        Scenario::Utils {
            payload,
            set_verbose,
            log_level,
            logger_only,
        } => healthy_hooks(set_verbose).utils_print_verbose(payload, log_level, &logger_only),
        Scenario::Active {
            set_verbose,
            logger_debug_enabled,
        } => {
            let hooks = hooks(
                FixtureLogger {
                    fails: false,
                    debug_enabled: logger_debug_enabled,
                },
                FixtureConsole::default(),
                set_verbose,
            );
            assert_eq!(hooks.is_active(), expected.active);
            return;
        }
    };
    assert_eq!(result.logs.len(), expected.logs);
    assert_eq!(result.console.len(), expected.console);
}

#[test]
fn redacts_object_credentials_before_any_sink_sees_them() {
    let result = healthy_hooks(json!(true)).main_print_verbose(credential_payload());
    assert_eq!(result.logs[0].payload["api_key"], "REDACTED");
    assert_eq!(result.console[0]["message"], "body");
}

#[test]
fn redacts_a_bearer_token_carried_as_a_bare_string() {
    let result = healthy_hooks(json!(true))
        .main_print_verbose(json!("Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz"));
    assert!(
        !result.logs[0]
            .payload
            .as_str()
            .unwrap()
            .contains("abcdefghijklmnopqrstuvwxyz")
    );
    assert!(
        !result.console[0]
            .as_str()
            .unwrap()
            .contains("abcdefghijklmnopqrstuvwxyz")
    );
}

#[test]
fn redacts_a_bearer_token_nested_inside_an_array() {
    let result = healthy_hooks(json!(true))
        .main_print_verbose(json!(["Bearer sk-abcdefghijklmnopqrstuvwxyz", "plain"]));
    let logged = result.logs[0].payload.as_array().unwrap();
    assert!(
        !logged[0]
            .as_str()
            .unwrap()
            .contains("abcdefghijklmnopqrstuvwxyz")
    );
    assert_eq!(logged[1], "plain");
}

#[test]
fn logger_failure_suppresses_both_sinks() {
    let failing_logger = || {
        hooks(
            FixtureLogger {
                fails: true,
                debug_enabled: false,
            },
            FixtureConsole::default(),
            json!(true),
        )
    };

    let main_failed = failing_logger().main_print_verbose(json!("body"));
    assert!(main_failed.logs.is_empty());
    assert!(main_failed.console.is_empty());

    let utils_failed = failing_logger().utils_print_verbose(json!("body"), "DEBUG", &json!(false));
    assert!(utils_failed.logs.is_empty());
    assert!(utils_failed.console.is_empty());
}

#[test]
fn logger_only_suppresses_the_console_even_when_verbose_is_set() {
    let result =
        healthy_hooks(json!(true)).utils_print_verbose(json!("body"), "DEBUG", &json!(true));
    assert_eq!(result.logs.len(), 1);
    assert!(result.console.is_empty());
}

#[test]
fn unknown_log_level_skips_the_logger_but_not_the_console() {
    let result =
        healthy_hooks(json!(true)).utils_print_verbose(json!("body"), "TRACE", &json!(false));
    assert!(result.logs.is_empty());
    assert_eq!(result.console.len(), 1);
}

#[test]
fn console_failure_does_not_drop_the_logger_record() {
    let result = hooks(
        FixtureLogger::default(),
        FixtureConsole { fails: true },
        json!(true),
    )
    .main_print_verbose(json!("body"));
    assert_eq!(result.logs.len(), 1);
    assert!(result.console.is_empty());
}

#[test]
fn main_guard_treats_a_truthy_non_boolean_set_verbose_as_verbose() {
    let result = healthy_hooks(json!(1_i64)).main_print_verbose(json!("body"));
    assert_eq!(result.console.len(), 1);
}

#[test]
fn utils_guard_requires_set_verbose_to_be_identically_true_not_merely_truthy() {
    let result =
        healthy_hooks(json!(1_i64)).utils_print_verbose(json!("body"), "DEBUG", &json!(false));
    assert!(result.console.is_empty());
}

#[test]
fn utils_guard_requires_logger_only_to_be_identically_false_a_falsey_non_boolean_suppresses_the_console()
 {
    let result =
        healthy_hooks(json!(true)).utils_print_verbose(json!("body"), "DEBUG", &json!(0_i64));
    assert!(result.console.is_empty());
}

#[test]
fn is_active_is_true_when_the_logger_is_already_enabled_at_debug_even_if_set_verbose_is_falsey() {
    let hooks = hooks(
        FixtureLogger {
            fails: false,
            debug_enabled: true,
        },
        FixtureConsole::default(),
        json!(0_i64),
    );
    assert!(hooks.is_active());
}

#[test]
fn is_active_requires_identity_not_truthiness_for_set_verbose() {
    assert!(!healthy_hooks(json!(1_i64)).is_active());
}
