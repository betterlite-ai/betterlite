#![cfg(test)]

use rstest::rstest;

use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_aware_routing::tests::support::PARAMS;
use crate::cache_aware_routing::tests::support::Resolution;
use crate::cache_aware_routing::tests::support::model_info;
use crate::cache_aware_routing::tests::support::params;
use crate::cache_aware_routing::tests::support::settings;
use crate::cache_prediction_targets::target::resolve_prediction_target;

fn supported(params_json: &str) -> bool {
    let litellm_params = params(params_json);
    let info = model_info();
    let deployment = PredictionDeployment::read(&litellm_params, &info);
    let resolution = Resolution::new();
    let context = resolution.context();
    resolve_prediction_target(
        deployment.litellm_params,
        &settings("https://api.anthropic.com"),
        &context,
    )
    .is_ok()
}

#[test]
fn a_native_anthropic_deployment_is_supported() {
    assert!(supported(PARAMS));
}

#[rstest]
#[case::extra_option(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","organization":"org"}"#
)]
#[case::non_public_base(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_base":"https://proxy.test"}"#
)]
#[case::oauth_key(r#"{"model":"anthropic/claude-x","api_key":"sk-ant-oat01-x"}"#)]
#[case::no_key(r#"{"model":"anthropic/claude-x"}"#)]
#[case::no_model(r#"{"api_key":"sk-ant-api03-x"}"#)]
fn an_unpredictable_deployment_is_not_supported(#[case] params_json: &str) {
    assert!(!supported(params_json));
}

#[test]
fn a_blocked_deployment_still_resolves_a_target() {
    let litellm_params = params(PARAMS);
    let info = params(r#"{"id":"deployment-a","blocked":true}"#);
    let deployment = PredictionDeployment::read(&litellm_params, &info);
    let resolution = Resolution::new();
    let context = resolution.context();
    assert!(
        resolve_prediction_target(
            deployment.litellm_params,
            &settings("https://api.anthropic.com"),
            &context,
        )
        .is_ok()
    );
}
