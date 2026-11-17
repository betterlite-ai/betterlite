#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rand::SeedableRng as _;
use rand::rngs::StdRng;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request, strategy_context,
};
use crate::static_eligibility::request::RequestFacts;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{Resolution, ResolutionSource};

use super::select;

fn spec(id: &str, kind: &str) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text("openai/probe".to_owned()));
    let info = JsonObject::from_iter([
        ("id".to_owned(), Json::Text(id.to_owned())),
        ("type".to_owned(), Json::Text(kind.to_owned())),
    ]);
    DeploymentSpec::new(
        "gpt-4o".to_owned(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

async fn selected(deployments: &[DeploymentSpec]) -> usize {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::Lar1, None).unwrap();
    let resolution = Resolution {
        strategy: RoutingStrategy::Lar1,
        selector: None,
        source: ResolutionSource::Default,
        bound: false,
        ignored_override: None,
    };
    let context = strategy_context(deployments, &route, &mut dispatch, &resolution);
    let mut random = StdRng::seed_from_u64(7);
    select(&context, &mut random).await.unwrap()
}

// the declared confidence of 0.5 classifies as local, so the arm takes the first local deployment
#[tokio::test]
async fn lar1_selects_the_first_deployment_of_the_default_type() {
    let deployments = [spec("smart", "cloud-smart"), spec("local", "local")];
    assert_eq!(selected(&deployments).await, 1);
}

#[tokio::test]
async fn lar1_falls_back_to_the_first_deployment() {
    let deployments = [spec("smart", "cloud-smart"), spec("deep", "deep")];
    assert_eq!(selected(&deployments).await, 0);
}

#[tokio::test]
async fn lar1_keeps_the_first_index_for_an_empty_pool() {
    let deployments: [DeploymentSpec; 0] = [];
    assert_eq!(selected(&deployments).await, 0);
}
