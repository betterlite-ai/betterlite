#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::DeploymentPricing;
use super::ModelInfo;
use super::RouterModelError;
use super::get_router_model_id;
use super::router_deployment_model_info;
use crate::logging_metadata::params::LitellmParams;

fn params(encoded: &str) -> LitellmParams {
    serde_json::from_str(encoded).expect("params decode")
}

struct ExpectedPricing {
    model_id: Option<&'static str>,
    deployment_model: Option<&'static str>,
    result: Option<ModelInfo>,
}

impl DeploymentPricing for ExpectedPricing {
    fn deployment_pricing_model_info(
        &self,
        model_id: Option<&str>,
        deployment_model: Option<&str>,
    ) -> Option<ModelInfo> {
        assert_eq!(model_id, self.model_id);
        assert_eq!(deployment_model, self.deployment_model);
        self.result.clone()
    }
}

struct UnreachablePricing;

impl DeploymentPricing for UnreachablePricing {
    fn deployment_pricing_model_info(
        &self,
        _model_id: Option<&str>,
        _deployment_model: Option<&str>,
    ) -> Option<ModelInfo> {
        panic!("pricing must not be asked for an id that does not resolve");
    }
}

fn model_info() -> ModelInfo {
    IndexMap::from([(
        String::from("input_cost_per_token"),
        Json::Number(serde_json::Number::from(1_i32)),
    )])
}

#[rstest]
#[case::litellm_metadata(
    r#"{"litellm_metadata": {"model_info": {"id": "model-1"}}}"#,
    Some("model-1")
)]
#[case::metadata(r#"{"metadata": {"model_info": {"id": "model-2"}}}"#, Some("model-2"))]
#[case::litellm_metadata_wins(r#"{"litellm_metadata": {"model_info": {"id": "model-1"}}, "metadata": {"model_info": {"id": "model-2"}}}"#, Some("model-1"))]
#[case::null_id_falls_through(r#"{"litellm_metadata": {"model_info": {"id": null}}, "metadata": {"model_info": {"id": "model-2"}}}"#, Some("model-2"))]
#[case::missing_model_info("{}", None)]
#[case::null_model_info(r#"{"metadata": {"model_info": null}}"#, None)]
fn logging_metadata_behavior_tests_behavior(#[case] encoded: &str, #[case] expected: Option<&str>) {
    let model_id = get_router_model_id(&params(encoded)).expect("model id");
    assert_eq!(model_id.as_deref(), expected);
}

#[test]
fn a_non_object_model_info_is_refused() {
    let error = get_router_model_id(&params(r#"{"metadata": {"model_info": "model-1"}}"#))
        .expect_err("wrong type");
    assert!(matches!(error, RouterModelError::ModelInfoType));
}

#[test]
fn a_non_text_model_id_is_refused() {
    let error = get_router_model_id(&params(r#"{"metadata": {"model_info": {"id": 7}}}"#))
        .expect_err("wrong type");
    assert!(matches!(error, RouterModelError::ModelIdType));
}

#[test]
fn the_deployment_info_query_passes_the_projected_id_and_model() {
    let pricing = ExpectedPricing {
        model_id: Some("model-1"),
        deployment_model: Some("openai/gpt-4o"),
        result: Some(model_info()),
    };
    let info = router_deployment_model_info(
        &pricing,
        &params(r#"{"litellm_metadata": {"model_info": {"id": "model-1"}}}"#),
        Some("openai/gpt-4o"),
    )
    .expect("deployment info");
    assert_eq!(info, Some(model_info()));
}

#[test]
fn an_unresolved_model_id_asks_pricing_with_no_id() {
    let pricing = ExpectedPricing {
        model_id: None,
        deployment_model: None,
        result: None,
    };
    let info =
        router_deployment_model_info(&pricing, &params("{}"), None).expect("deployment info");
    assert_eq!(info, None);
}

#[test]
fn a_malformed_model_id_stops_before_pricing() {
    let error = router_deployment_model_info(
        &UnreachablePricing,
        &params(r#"{"metadata": {"model_info": {"id": []}}}"#),
        None,
    )
    .expect_err("wrong type");
    assert!(matches!(error, RouterModelError::ModelIdType));
}
