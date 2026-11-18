#![cfg(test)]

use crate::roi_settings::deployments::RouterEstimatorDeployment;

#[test]
fn deployment_parses_the_litellm_params_and_ignores_unknown_fields() {
    let deployment: RouterEstimatorDeployment = serde_json::from_str(
        r#"{"litellm_params":{"model":"gpt-4o","base_model":null,"custom_llm_provider":"openai","extra":1},"model_info":{"base_model":"gpt-4o-mini","extra":2},"extra":3}"#,
    )
    .expect("deployment json");
    assert_eq!(deployment.litellm_params.model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        deployment.litellm_params.custom_llm_provider.as_deref(),
        Some("openai")
    );
    assert_eq!(
        deployment
            .model_info
            .as_ref()
            .and_then(|info| info.base_model.as_deref()),
        Some("gpt-4o-mini")
    );
}

#[test]
fn deployment_without_model_info_or_with_a_null_one_defaults_to_none() {
    let absent: RouterEstimatorDeployment =
        serde_json::from_str(r#"{"litellm_params":{"model":"gpt-4o"}}"#).expect("deployment json");
    assert!(absent.model_info.is_none());
    let explicit: RouterEstimatorDeployment =
        serde_json::from_str(r#"{"litellm_params":{"model":"gpt-4o"},"model_info":null}"#)
            .expect("null model_info json");
    assert!(explicit.model_info.is_none());
}

#[test]
fn deployment_without_litellm_params_is_rejected() {
    serde_json::from_str::<RouterEstimatorDeployment>(r#"{"model_info":{}}"#).unwrap_err();
}
