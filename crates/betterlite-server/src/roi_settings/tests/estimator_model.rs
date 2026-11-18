#![cfg(test)]

use rstest::rstest;

use crate::roi_settings::deployments::RouterEstimatorDeployment;
use crate::roi_settings::deployments::RouterEstimatorModelInfo;
use crate::roi_settings::deployments::RouterEstimatorParams;
use crate::roi_settings::deployments::estimator_model;

fn deployment(
    model_info_base: Option<&str>,
    parameters_base: Option<&str>,
    model: Option<&str>,
) -> RouterEstimatorDeployment {
    RouterEstimatorDeployment {
        litellm_params: RouterEstimatorParams {
            model: model.map(str::to_owned),
            base_model: parameters_base.map(str::to_owned),
            custom_llm_provider: Some("openai".to_owned()),
        },
        model_info: model_info_base.map(|base| RouterEstimatorModelInfo {
            base_model: Some(base.to_owned()),
        }),
    }
}

#[rstest]
#[case::model_info_wins(Some("info"), Some("params"), Some("model"), Some("info"))]
#[case::base_model_without_model_info(None, Some("params"), Some("model"), Some("params"))]
#[case::plain_model(None, None, Some("model"), Some("model"))]
#[case::nothing_set(None, None, None, None)]
#[case::empty_string_is_used_as_given(None, Some(""), Some("model"), Some(""))]
fn estimator_model_prefers_the_most_specific_model(
    #[case] info: Option<&str>,
    #[case] parameters_base: Option<&str>,
    #[case] model: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let found = estimator_model(&deployment(info, parameters_base, model));
    assert_eq!(
        found.map(|estimator| estimator.0),
        expected.map(str::to_owned)
    );
}

#[test]
fn estimator_model_falls_through_a_model_info_without_a_base_model() {
    let mut deployment = deployment(None, Some("params"), Some("model"));
    deployment.model_info = Some(RouterEstimatorModelInfo { base_model: None });
    assert_eq!(
        estimator_model(&deployment).map(|estimator| estimator.0),
        Some("params".to_owned())
    );
}

#[test]
fn estimator_model_keeps_the_custom_provider() {
    let found = estimator_model(&deployment(None, None, Some("gpt-4o"))).expect("a model");
    assert_eq!(found.1.as_deref(), Some("openai"));
}
