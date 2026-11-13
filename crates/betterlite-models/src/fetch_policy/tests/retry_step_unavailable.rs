use super::support;
use crate::fetch_policy::outcome::{AttemptOutcome, FetchResult, Unavailable};
use crate::fetch_policy::retry::{NextStep, next_retry_step};

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let mut source = support::ScriptedJitter::new(&[]);
    let step = next_retry_step(
        AttemptOutcome::Unavailable(Unavailable {
            reason: "HTTP 404".to_owned(),
        }),
        1,
        3,
        &mut source,
    );
    match step {
        NextStep::Done(FetchResult::Unavailable(unavailable)) => {
            assert_eq!(unavailable.reason, "HTTP 404");
        }
        NextStep::Done(FetchResult::Reloaded(_)) | NextStep::Retry(_) => {
            panic!("an unavailable outcome must not retry");
        }
    }
}
