use super::support;
use crate::fetch_policy::outcome::AttemptOutcome;
use crate::fetch_policy::retry::{NextStep, next_retry_step};

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let mut source = support::ScriptedJitter::new(&["0.25"]);
    match next_retry_step(
        AttemptOutcome::Retryable(support::retryable(None)),
        1,
        3,
        &mut source,
    ) {
        NextStep::Retry(seconds) => assert_eq!(seconds, support::decimal("2.25")),
        NextStep::Done(_) => panic!("a retryable outcome below the last attempt must retry"),
    }
}
