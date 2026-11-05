#![cfg(test)]
use super::{failure_row_traceback, should_suppress_tracebacks};
use betterlite_config::collector::{TracebackPolicy, load_spend_traceback_policy};
use tokio::sync::watch;

#[test]
fn loads_spend_traceback_policy() {
    let (sender, snapshot) = watch::channel(TracebackPolicy::default());
    assert!(!should_suppress_tracebacks(&snapshot, false));
    sender.send_replace(load_spend_traceback_policy(Some(" TRUE ")).unwrap());
    assert!(should_suppress_tracebacks(&snapshot, false));
    assert!(!should_suppress_tracebacks(&snapshot, true));
    assert_eq!(
        failure_row_traceback(&snapshot, Some("trace".to_owned()), false),
        None
    );
    assert_eq!(
        failure_row_traceback(&snapshot, Some("trace".to_owned()), true),
        Some("trace".to_owned())
    );
    sender.send_replace(load_spend_traceback_policy(None).unwrap());
    assert!(!should_suppress_tracebacks(&snapshot, false));
}

#[rstest::rstest]
#[case::yes("yes", true)]
#[case::one("1", true)]
#[case::false_value("false", false)]
#[case::empty("", false)]
fn follows_shared_environment_boolean_policy(#[case] value: &str, #[case] expected: bool) {
    assert_eq!(
        load_spend_traceback_policy(Some(value)).unwrap().suppress,
        expected
    );
    load_spend_traceback_policy(Some("invalid")).unwrap_err();
}
