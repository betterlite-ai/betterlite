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
use crate::strategy_dispatch::args::{LatencyArgs, SelectorArgs};
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{Resolution, ResolutionSource};

use super::select;

fn deployment(id: &str, tpm: Option<Json>) -> DeploymentSpec {
    let mut extra = JsonObject::new();
    if let Some(limit) = tpm {
        extra.insert("tpm".to_owned(), limit);
    }
    let params =
        JsonObject::from_iter([("model".to_owned(), Json::Text("openai/probe".to_owned()))]);
    let info = JsonObject::from_iter([("id".to_owned(), Json::Text(id.to_owned()))]);
    DeploymentSpec::new("gpt-4o".to_owned(), params, info, extra, String::new)
        .expect("the fixture deployment is valid")
}

fn tokens(count: usize) -> RequestFacts {
    RequestFacts {
        messages: None,
        input: None,
        instructions: None,
        system: None,
        tools: Vec::new(),
        response_format: false,
        target_order: None,
        input_token_count: Some(count),
        compaction_pending: false,
    }
}

async fn run(
    deployments: &[DeploymentSpec],
    facts: &RequestFacts,
    args: LatencyArgs,
    seed: u64,
) -> Result<usize, RoutingError> {
    let request = route_request("gpt-4o", facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let mut dispatch = StrategyDispatch::new(
        RoutingStrategy::LatencyBasedRouting,
        Some(SelectorArgs::Latency(args)),
    )
    .expect("the fixture arguments are latency arguments");
    let resolution = Resolution {
        strategy: RoutingStrategy::LatencyBasedRouting,
        selector: None,
        source: ResolutionSource::Default,
        bound: false,
        ignored_override: None,
    };
    let context = strategy_context(deployments, &route, &mut dispatch, &resolution);
    let mut random = StdRng::seed_from_u64(seed);
    select(&context, &mut random).await
}

// the arm reads an empty sample map, so every deployment scores zero and any pool index may win
#[tokio::test]
async fn latency_routing_arm_picks_from_the_pool() {
    let deployments = [deployment("a", None), deployment("b", None)];
    let selected = run(&deployments, &tokens(0), LatencyArgs::default(), 7)
        .await
        .expect("the fixture limits are numbers");
    assert!(selected < deployments.len());
}

// the fastest deployment has no token room, so the one that does answers whatever the draw
#[tokio::test]
async fn latency_routing_arm_skips_a_rate_limited_deployment() {
    let deployments = [
        deployment("fast", Some(Json::Number(Number::from(0_i64)))),
        deployment("slow", None),
    ];
    let selected = run(&deployments, &tokens(5), LatencyArgs::default(), 7)
        .await
        .expect("the fixture limits are numbers");
    assert_eq!(selected, 1);
}

// an index past the pool is how the pipeline reaches its no-deployment error
#[tokio::test]
async fn latency_routing_arm_answers_no_deployment_when_none_fit() {
    let deployments = [deployment("full", Some(Json::Number(Number::from(0_i64))))];
    let selected = run(&deployments, &tokens(5), LatencyArgs::default(), 7)
        .await
        .expect("the fixture limits are numbers");
    assert_eq!(selected, deployments.len());
}

#[tokio::test]
async fn latency_routing_arm_reports_a_limit_that_is_no_number() {
    let deployments = [deployment("odd", Some(Json::Text("5".to_owned())))];
    let error = run(&deployments, &tokens(5), LatencyArgs::default(), 7)
        .await
        .expect_err("the fixture limit is no number");
    assert!(
        matches!(error, RoutingError::Metric { field: "tpm" }),
        "the arm answered {error:?}"
    );
}
