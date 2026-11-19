#![cfg(test)]

use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_aware_routing::tests::support::FakeCost;
use crate::cache_aware_routing::tests::support::FakeMinimum;
use crate::cache_aware_routing::tests::support::MemoryCache;
use crate::cache_aware_routing::tests::support::PARAMS;
use crate::cache_aware_routing::tests::support::Resolution;
use crate::cache_aware_routing::tests::support::model_info;
use crate::cache_aware_routing::tests::support::params;
use crate::cache_aware_routing::tests::support::prediction_services;
use crate::cache_aware_routing::tests::support::settings;

fn supported_info(params_json: &str, info: &JsonObject) -> bool {
    let litellm_params = params(params_json);
    let deployment = PredictionDeployment::read(&litellm_params, info);
    let resolution = Resolution::new();
    let context = resolution.context();
    let settings = settings("https://api.anthropic.com");
    let cache = MemoryCache::new();
    let cost = FakeCost::new(None);
    let minimum = FakeMinimum { tokens: 0 };
    let services = prediction_services(&settings, &context, &cache, &cost, &minimum);
    AnthropicCacheRouting::supports(&deployment, &services)
}

fn supported(params_json: &str) -> bool {
    supported_info(params_json, &model_info())
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
fn a_blocked_deployment_is_still_supported() {
    let info = params(r#"{"id":"deployment-a","blocked":true}"#);
    assert!(supported_info(PARAMS, &info));
}
