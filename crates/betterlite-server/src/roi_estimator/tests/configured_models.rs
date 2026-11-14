#![cfg(test)]

use betterlite_roi::estimator::models::EstimatorModel;
use betterlite_roi::estimator::options::configured_models;

use super::support::settings;

#[test]
fn configured_models_substitutes_the_estimator_model_when_models_is_none() {
    let settings = settings("gpt-5.2", "Estimate this.");
    assert_eq!(
        configured_models(&settings, None),
        vec![("gpt-5.2".to_owned(), None)]
    );
}

#[test]
fn configured_models_returns_an_empty_tuple_unchanged() {
    let settings = settings("gpt-5.2", "Estimate this.");
    assert!(configured_models(&settings, Some(&[])).is_empty());
}

#[test]
fn configured_models_returns_the_passed_models_unchanged() {
    let settings = settings("unused", "Estimate this.");
    let models: Vec<EstimatorModel> = vec![
        ("gpt-5.2".to_owned(), Some("openai".to_owned())),
        ("deepseek-chat".to_owned(), None),
    ];
    assert_eq!(configured_models(&settings, Some(&models)), models);
}
