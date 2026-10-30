#![cfg(test)]

use rstest::rstest;
use serde_json::json;

use super::{HookInput, VerboseHooks};

#[rstest]
#[case::main_verbose_guard(HookInput::Main { payload: json!("message"), set_verbose: true, logger_fails: false, console_fails: false }, 1, 1)]
#[case::utils_verbose_levels(HookInput::Utils { payload: json!("message"), set_verbose: true, log_level: "INFO".to_owned(), logger_only: false, logger_fails: false, console_fails: false }, 1, 1)]
#[case::verbose_active(HookInput::Active { set_verbose: false, logger_debug_enabled: true }, 0, 0)]
#[case::redacted_verbose_payload(HookInput::Main { payload: json!("message"), set_verbose: true, logger_fails: false, console_fails: false }, 1, 1)]
fn verbose_hooks_cases(#[case] input: HookInput, #[case] logs: usize, #[case] console: usize) {
    let result = VerboseHooks.run(input);
    assert_eq!(result.logs.len(), logs);
    assert_eq!(result.console.len(), console);
}

#[test]
fn verbose_hooks_cases_redact_and_suppress_failures() {
    let input = HookInput::Main {
        payload: json!({"api_key": "secret", "message": "body"}),
        set_verbose: true,
        logger_fails: false,
        console_fails: false,
    };
    let result = VerboseHooks.run(input);
    assert_eq!(result.logs[0].payload["api_key"], "REDACTED");
    assert_eq!(result.console[0]["message"], "body");

    let failed = VerboseHooks.run(HookInput::Main {
        payload: json!("body"),
        set_verbose: true,
        logger_fails: true,
        console_fails: false,
    });
    assert!(failed.logs.is_empty());
    assert!(failed.console.is_empty());
}
