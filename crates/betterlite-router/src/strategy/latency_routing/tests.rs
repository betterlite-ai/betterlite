#![cfg(test)]

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

#[tokio::test]
async fn router_core_boundaries() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::LatencyBasedRouting, None).unwrap();
    let resolution = Resolution {
        strategy: RoutingStrategy::LatencyBasedRouting,
        selector: None,
        source: ResolutionSource::Default,
        bound: false,
        ignored_override: None,
    };
    let deployments: [DeploymentSpec; 0] = [];
    let context = strategy_context(&deployments, &route, &mut dispatch, &resolution);
    let mut random = StdRng::seed_from_u64(7);
    let error = select(&context, &mut random).await.unwrap_err();
    assert_eq!(error.unbuilt_entry(), Some("latency_routing"));
}
