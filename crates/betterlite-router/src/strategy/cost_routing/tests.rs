#![cfg(test)]

use std::str::FromStr as _;

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

const INPUT_COST: &str = "input_cost_per_token";
const OUTPUT_COST: &str = "output_cost_per_token";
const TPM: &str = "tpm";

fn number(value: &str) -> Json {
    Json::Number(Number::from_str(value).unwrap())
}

fn spec(id: &str, params: &[(&str, Json)]) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), Json::Text("openai/probe".to_owned()));
    for (key, value) in params {
        litellm_params.insert((*key).to_owned(), value.clone());
    }
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new(
        "probe-group".to_owned(),
        litellm_params,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn priced(id: &str, input: &str, output: &str) -> DeploymentSpec {
    spec(
        id,
        &[(INPUT_COST, number(input)), (OUTPUT_COST, number(output))],
    )
}

async fn selected(
    deployments: &[DeploymentSpec],
    facts: &RequestFacts,
) -> Result<usize, RoutingError> {
    let request = route_request("probe-group", facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::CostBasedRouting, None).unwrap();
    let resolution = Resolution {
        strategy: RoutingStrategy::CostBasedRouting,
        selector: None,
        source: ResolutionSource::Default,
        bound: false,
        ignored_override: None,
    };
    let context = strategy_context(deployments, &route, &mut dispatch, &resolution);
    let mut random = StdRng::seed_from_u64(7);
    select(&context, &mut random).await
}

#[tokio::test]
async fn cheapest_custom_price_wins() {
    let deployments = [priced("a", "0.3", "0.3"), priced("b", "0.01", "0.02")];
    let facts = RequestFacts::default();
    assert_eq!(selected(&deployments, &facts).await, Ok(1));
}

#[tokio::test]
async fn tpm_limit_screens_the_cheapest_deployment() {
    let deployments = [
        spec("a", &[(INPUT_COST, number("0.01")), (TPM, number("100"))]),
        priced("b", "0.3", "0.3"),
    ];
    let facts = RequestFacts {
        input_token_count: Some(101),
        ..RequestFacts::default()
    };
    assert_eq!(selected(&deployments, &facts).await, Ok(1));
}

#[tokio::test]
async fn equal_prices_keep_the_pool_order() {
    let deployments = [priced("a", "0.1", "0.1"), priced("b", "0.1", "0.1")];
    let facts = RequestFacts::default();
    assert_eq!(selected(&deployments, &facts).await, Ok(0));
}

// a selection with nothing left answers the pool length, which the pipeline turns into its no-deployment error
#[tokio::test]
async fn every_deployment_full_answers_past_the_pool() {
    let deployments = [
        spec("a", &[(TPM, number("1"))]),
        spec("b", &[(TPM, number("1"))]),
    ];
    let facts = RequestFacts {
        input_token_count: Some(10),
        ..RequestFacts::default()
    };
    assert_eq!(selected(&deployments, &facts).await, Ok(2));
}

#[tokio::test]
async fn empty_pool_answers_the_first_index() {
    let facts = RequestFacts::default();
    assert_eq!(selected(&[], &facts).await, Ok(0));
}

#[tokio::test]
async fn non_numeric_price_names_its_field() {
    let deployments = [spec("a", &[(INPUT_COST, Json::Text("cheap".to_owned()))])];
    let facts = RequestFacts::default();
    let error = selected(&deployments, &facts).await.unwrap_err();
    let RoutingError::Metric { field } = error else {
        panic!("a non-numeric price is a metric error");
    };
    assert_eq!(field, INPUT_COST);
}
