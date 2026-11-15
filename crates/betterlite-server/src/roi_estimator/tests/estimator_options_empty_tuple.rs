#![cfg(test)]

use betterlite_roi::estimator::options::estimator_options;

use super::support::ModelSupport;

#[tokio::test]
async fn estimator_options_returns_empty_options_for_an_empty_models_tuple() {
    let mut predicates = ModelSupport::supporting(&["gpt-5.2"]);
    let options = estimator_options(&mut predicates, &[]).await;
    assert_eq!(options.reasoning_effort, None);
    assert!(predicates.seen.is_empty());
}
