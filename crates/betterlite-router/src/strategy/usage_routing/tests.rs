#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use serde_json::Number;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::error::RoutingError;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request, strategy_context,
};
use crate::static_eligibility::request::RequestFacts;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{Resolution, ResolutionSource};

use super::select;

fn spec(id: &str, limits: &[(&str, Json)]) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text("openai/probe".to_owned()));
    let mut info = JsonObject::from_iter([("id".to_owned(), Json::Text(id.to_owned()))]);
    for (key, value) in limits {
        info.insert((*key).to_owned(), value.clone());
    }
    DeploymentSpec::new(
        "gpt-4o".to_owned(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

async fn outcome(
    deployments: &[DeploymentSpec],
    tokens: Option<usize>,
) -> Result<usize, RoutingError> {
    let facts = RequestFacts {
        input_token_count: tokens,
        ..RequestFacts::default()
    };
    let request = route_request("gpt-4o", &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::UsageBasedRouting, None).unwrap();
    let resolution = Resolution {
        strategy: RoutingStrategy::UsageBasedRouting,
        selector: None,
        source: ResolutionSource::Default,
        bound: false,
        ignored_override: None,
    };
    let context = strategy_context(deployments, &route, &mut dispatch, &resolution);
    let mut random = StdRng::seed_from_u64(7);
    select(&context, &mut random).await
}

// the pool order is the tie order of a cold window, so the first deployment is the one it picks
#[tokio::test]
async fn a_cold_window_picks_the_first_deployment() {
    let deployments = [spec("dep-a", &[]), spec("dep-b", &[])];
    assert_eq!(outcome(&deployments, None).await.unwrap(), 0);
    assert_eq!(outcome(&deployments, Some(10)).await.unwrap(), 0);
}

#[tokio::test]
async fn a_cold_window_skips_a_deployment_the_estimate_cannot_fit() {
    let deployments = [spec("dep-a", &[("tpm", number(5))]), spec("dep-b", &[])];
    assert_eq!(outcome(&deployments, Some(10)).await.unwrap(), 1);
}

// the selector always answers an index of the pool, and one past it is the pipeline's no-deployment path
#[tokio::test]
async fn a_cold_window_with_every_deployment_ruled_out_names_an_index_past_the_pool() {
    let deployments = [
        spec("dep-a", &[("tpm", number(0))]),
        spec("dep-b", &[("tpm", number(0))]),
    ];
    assert_eq!(
        outcome(&deployments, Some(1)).await.unwrap(),
        deployments.len()
    );
}

#[tokio::test]
async fn an_empty_pool_keeps_index_zero() {
    let deployments: [DeploymentSpec; 0] = [];
    assert_eq!(outcome(&deployments, None).await.unwrap(), 0);
}

#[tokio::test]
async fn an_absent_estimate_counts_as_no_tokens() {
    let deployments = [spec("dep-a", &[("tpm", number(0))]), spec("dep-b", &[])];
    assert_eq!(outcome(&deployments, None).await.unwrap(), 0);
    assert_eq!(outcome(&deployments, Some(1)).await.unwrap(), 1);
}

// a count wider than i64 saturates instead of wrapping to a negative that no limit would rule out
#[tokio::test]
async fn a_huge_estimate_saturates_at_the_signed_maximum() {
    let deployments = [spec("dep-a", &[("tpm", number(i64::MAX / 2))])];
    assert_eq!(
        outcome(&deployments, Some(usize::MAX)).await.unwrap(),
        deployments.len()
    );
}

#[tokio::test]
async fn the_rpm_limit_never_rules_out_a_cold_window() {
    let deployments = [spec("dep-a", &[("rpm", number(0))])];
    assert_eq!(outcome(&deployments, None).await.unwrap(), 0);
}

#[tokio::test]
async fn a_limit_of_the_wrong_type_names_the_field() {
    let text = [spec("dep-a", &[("tpm", Json::Text("100".to_owned()))])];
    let boolean = [spec("dep-a", &[("rpm", Json::Bool(true))])];
    let tpm = outcome(&text, None).await.unwrap_err();
    let rpm = outcome(&boolean, None).await.unwrap_err();
    assert!(matches!(tpm, RoutingError::Metric { field: "tpm" }));
    assert!(matches!(rpm, RoutingError::Metric { field: "rpm" }));
}
