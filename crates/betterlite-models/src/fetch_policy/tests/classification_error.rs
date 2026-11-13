use super::support;
use crate::fetch_policy::outcome::{AttemptOutcome, classify_fetch_error};

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let invalid_url = support::ScriptedError::InvalidUrl {
        detail: "unsupported scheme".to_owned(),
    };
    match classify_fetch_error(&invalid_url, support::URL) {
        AttemptOutcome::Unavailable(unavailable) => assert_eq!(
            unavailable.reason,
            format!("unsupported scheme fetching {}", support::URL)
        ),
        AttemptOutcome::Reloaded(_) | AttemptOutcome::Retryable(_) => {
            panic!("an invalid URL is unavailable");
        }
    }

    let transport = support::ScriptedError::Transport {
        detail: "connection refused".to_owned(),
    };
    match classify_fetch_error(&transport, support::URL) {
        AttemptOutcome::Retryable(retryable) => {
            assert_eq!(
                retryable.reason,
                format!("connection refused fetching {}", support::URL)
            );
            assert_eq!(retryable.retry_after_seconds, None);
        }
        AttemptOutcome::Reloaded(_) | AttemptOutcome::Unavailable(_) => {
            panic!("a transport error is retryable");
        }
    }
}
