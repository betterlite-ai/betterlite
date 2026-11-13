use super::support;
use crate::fetch_policy::outcome::{AttemptOutcome, FetchResult};
use crate::fetch_policy::retry::{NextStep, next_retry_step};

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let mut source = support::ScriptedJitter::new(&[]);
    match next_retry_step(
        AttemptOutcome::Retryable(support::retryable(None)),
        3,
        3,
        &mut source,
    ) {
        NextStep::Done(FetchResult::Unavailable(unavailable)) => {
            assert!(unavailable.reason.ends_with("(after 3 attempts)"));
            assert!(unavailable.reason.starts_with("HTTP 503 from"));
        }
        NextStep::Done(FetchResult::Reloaded(_)) | NextStep::Retry(_) => {
            panic!("the last attempt must not retry");
        }
    }
}
