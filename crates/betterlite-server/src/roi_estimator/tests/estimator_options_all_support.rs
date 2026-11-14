#![cfg(test)]

use betterlite_roi::estimator::models::{EstimatorModel, ReasoningEffort};
use betterlite_roi::estimator::options::estimator_options;

use super::support::ModelSupport;

#[tokio::test]
async fn estimator_options_enables_none_only_when_every_model_supports_it() {
    let models: Vec<EstimatorModel> = vec![
        ("gpt-5.2".to_owned(), Some("openai".to_owned())),
        ("deepseek-chat".to_owned(), None),
    ];
    let mut predicates = ModelSupport::supporting(&["gpt-5.2", "deepseek-chat"]);
    let options = estimator_options(&mut predicates, &models).await;
    assert_eq!(options.reasoning_effort, Some(ReasoningEffort::None));
    assert_eq!(
        predicates.seen,
        vec![
            (
                "supports_none_reasoning_effort".to_owned(),
                "gpt-5.2".to_owned(),
                Some("openai".to_owned())
            ),
            (
                "supports_none_reasoning_effort".to_owned(),
                "deepseek-chat".to_owned(),
                None
            ),
        ]
    );
}

#[tokio::test]
async fn estimator_options_falls_back_when_a_later_model_is_unsupported() {
    let models: Vec<EstimatorModel> = vec![
        ("gpt-5.2".to_owned(), None),
        ("deepseek-chat".to_owned(), None),
    ];
    let mut predicates = ModelSupport::supporting(&["gpt-5.2"]);
    let options = estimator_options(&mut predicates, &models).await;
    assert_eq!(options.reasoning_effort, None);
}

#[tokio::test]
async fn estimator_options_falls_back_when_an_earlier_model_is_unsupported() {
    let models: Vec<EstimatorModel> = vec![
        ("deepseek-chat".to_owned(), None),
        ("gpt-5.2".to_owned(), None),
    ];
    let mut predicates = ModelSupport::supporting(&["gpt-5.2"]);
    let options = estimator_options(&mut predicates, &models).await;
    assert_eq!(options.reasoning_effort, None);
    assert_eq!(predicates.seen.len(), 1);
}
