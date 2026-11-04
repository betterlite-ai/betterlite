#![cfg(test)]

use super::*;

#[test]
fn loads_spend_traceback_policy() {
    assert!(!loads_spend_traceback_policy(None));
    assert!(loads_spend_traceback_policy(Some("true")));
    assert!(!loads_spend_traceback_policy(Some("false")));
}

#[test]
fn debug_logging_overrides_traceback_suppression() {
    SUPPRESS_TRACEBACKS.store(loads_spend_traceback_policy(Some("true")), Ordering::SeqCst);
    assert!(should_suppress_tracebacks(false));
    assert!(!should_suppress_tracebacks(true));
    assert_eq!(
        failure_row_traceback(Some("trace".to_owned()), true),
        Some("trace".to_owned())
    );
    SUPPRESS_TRACEBACKS.store(loads_spend_traceback_policy(None), Ordering::SeqCst);
}
