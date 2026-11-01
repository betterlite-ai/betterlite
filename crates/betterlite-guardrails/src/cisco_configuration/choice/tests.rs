#![cfg(test)]

use std::sync::Arc;
use std::{env, fs, process};

use betterlite_config::sections::cisco_verdict_contracts::{
    FallbackAction, FlaggedAction, InspectionType,
};
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::cisco_configuration::choice::{
    DEFAULT_TIMEOUT_SECONDS, TimeoutValue, coerce_timeout, resolve_choice,
};

#[test]
fn resolve_choice_accepts_an_explicit_valid_value() {
    let resolved = resolve_choice::<InspectionType>(
        Some("mcp"),
        "BETTERLITE_CISCO_CHOICE_TEST_UNUSED",
        &["chat", "mcp"],
        InspectionType::Chat,
        "inspection_type",
    );
    assert_eq!(resolved, InspectionType::Mcp);
}

#[test]
fn resolve_choice_falls_back_to_the_environment_variable_when_value_is_absent() {
    let var = "BETTERLITE_CISCO_CHOICE_TEST_ENV_FALLBACK";
    let resolved = temp_env::with_var(var, Some("monitor"), || {
        resolve_choice::<FlaggedAction>(
            None,
            var,
            &["block", "monitor"],
            FlaggedAction::Block,
            "on_flagged_action",
        )
    });
    assert_eq!(resolved, FlaggedAction::Monitor);
}

#[test]
fn resolve_choice_returns_default_when_value_and_env_var_are_both_absent() {
    let var = "BETTERLITE_CISCO_CHOICE_TEST_BOTH_ABSENT";
    let resolved = temp_env::with_var_unset(var, || {
        resolve_choice::<FallbackAction>(
            None,
            var,
            &["allow", "block"],
            FallbackAction::Block,
            "fallback_on_error",
        )
    });
    assert_eq!(resolved, FallbackAction::Block);
}

#[test]
fn resolve_choice_returns_pinned_default_when_value_is_invalid() {
    let resolved = resolve_choice::<InspectionType>(
        Some("not-a-surface"),
        "BETTERLITE_CISCO_CHOICE_TEST_UNUSED",
        &["chat", "mcp"],
        InspectionType::Chat,
        "inspection_type",
    );
    assert_eq!(resolved, InspectionType::Chat);
}

#[test]
fn resolve_choice_warns_once_falling_back_from_an_invalid_value() {
    let log_path = env::temp_dir().join(format!("betterlite-cisco-choice-warn-{}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let guard = set_default(subscriber);
    let resolved = resolve_choice::<InspectionType>(
        Some("bogus"),
        "BETTERLITE_CISCO_CHOICE_TEST_UNUSED",
        &["chat", "mcp"],
        InspectionType::Chat,
        "inspection_type",
    );
    drop(guard);
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert_eq!(resolved, InspectionType::Chat);
    assert!(written.contains("bogus"));
    assert!(written.contains("inspection_type"));
}

#[rstest]
#[case::whole_number(TimeoutValue::Number(30.0_f64), Some(30.0_f64))]
#[case::text_number(TimeoutValue::Text("45"), Some(45.0_f64))]
#[case::below_minimum_clamps_up(TimeoutValue::Number(0.1_f64), Some(1.0_f64))]
#[case::above_maximum_clamps_down(TimeoutValue::Number(600.0_f64), Some(60.0_f64))]
#[case::unparsable_text(TimeoutValue::Text("not-a-number"), None)]
fn coerce_timeout_clamps_or_rejects(
    #[case] value: TimeoutValue<'_>,
    #[case] expected: Option<f64>,
) {
    assert_eq!(coerce_timeout(value), expected);
}

#[test]
fn coerce_timeout_warns_on_unparsable_text_and_default_stays_available() {
    let log_path = env::temp_dir().join(format!("betterlite-cisco-timeout-warn-{}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let guard = set_default(subscriber);
    let resolved = coerce_timeout(TimeoutValue::Text("nope"));
    drop(guard);
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert_eq!(resolved, None);
    assert!((DEFAULT_TIMEOUT_SECONDS - 10.0_f64).abs() < f64::EPSILON);
    assert!(written.contains("timeout"));
}
