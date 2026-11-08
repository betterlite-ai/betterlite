use super::support;
use crate::fetch_policy::outcome::{AttemptOutcome, FetchResult, FetchedCatalog};
use crate::fetch_policy::retry::{NextStep, next_retry_step};

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    let fetched = FetchedCatalog {
        body: Vec::new(),
        models: support::models(&["model"]),
        revision: "revision".to_owned(),
        etag: None,
    };
    let mut source = support::ScriptedJitter::new(&[]);
    match next_retry_step(AttemptOutcome::Reloaded(fetched), 1, 3, &mut source) {
        NextStep::Done(FetchResult::Reloaded(reloaded)) => {
            assert_eq!(reloaded.revision, "revision");
        }
        NextStep::Done(FetchResult::Unavailable(_)) | NextStep::Retry(_) => {
            panic!("a reloaded outcome is final");
        }
    }
}
