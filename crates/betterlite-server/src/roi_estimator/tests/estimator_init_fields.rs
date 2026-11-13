#![cfg(test)]

use betterlite_roi::estimator::models::EstimatorModel;

use crate::roi_estimator::Estimator;

use super::support::{ModelSupport, OK_CONTENT, caller, ok_response, settings};

#[tokio::test]
async fn estimator_init_keeps_settings_callback_and_substituted_models() {
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, None);
    assert_eq!(estimator.settings.estimator_model, "gpt-5.2");
    assert_eq!(estimator.models, vec![("gpt-5.2".to_owned(), None)]);
    let mut predicates = ModelSupport::none();
    estimator
        .estimate(&super::support::pull(), &mut predicates)
        .await
        .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn estimator_init_keeps_passed_models_unchanged() {
    let (complete, _requests) = caller(Ok(ok_response(OK_CONTENT)));
    let models: Vec<EstimatorModel> = vec![
        ("gpt-5.2".to_owned(), Some("openai".to_owned())),
        ("deepseek-chat".to_owned(), None),
    ];
    let estimator = Estimator::new(
        settings("unused", "Estimate this."),
        complete,
        Some(&models),
    );
    assert_eq!(estimator.models, models);
}
