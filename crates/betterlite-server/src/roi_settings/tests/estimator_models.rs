#![cfg(test)]

use crate::roi_settings::deployments::RouterEstimatorDeployment;
use crate::roi_settings::deployments::RouterEstimatorParams;
use crate::roi_settings::deployments::estimator_models;
use crate::roi_settings::support::deployment;

#[test]
fn estimator_models_skips_deployments_without_a_model() {
    let deployments = [
        deployment(Some("gpt-4o"), None),
        RouterEstimatorDeployment {
            litellm_params: RouterEstimatorParams {
                model: None,
                base_model: None,
                custom_llm_provider: Some("azure".to_owned()),
            },
            model_info: None,
        },
    ];
    assert_eq!(
        estimator_models(&deployments),
        vec![("gpt-4o".to_owned(), Some("openai".to_owned()))]
    );
}

#[test]
fn estimator_models_keeps_every_deployment_with_a_model() {
    let deployments = [
        deployment(Some("a"), None),
        deployment(Some("b"), Some("base-b")),
    ];
    assert_eq!(
        estimator_models(&deployments),
        vec![
            ("a".to_owned(), Some("openai".to_owned())),
            ("base-b".to_owned(), Some("openai".to_owned())),
        ]
    );
}
