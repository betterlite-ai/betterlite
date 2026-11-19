#![cfg(test)]

use std::sync::Arc;

use rand::SeedableRng as _;
use rand::rngs::StdRng;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::least_busy::identity::request_count_key;
use crate::least_busy::{IN_FLIGHT_COUNT_TTL_SECONDS, LeastBusy};
use crate::router_core::tests::support::{
    Counted, NoLimits, NoServices, Providers, group_and_ids, in_flight_store, route_context,
    route_request, strategy_context,
};
use crate::routing_value_store::RouteValue;
use crate::static_eligibility::request::RequestFacts;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{Resolution, ResolutionSource};

use super::select;

fn resolution() -> Resolution {
    Resolution {
        strategy: RoutingStrategy::LeastBusy,
        selector: None,
        source: ResolutionSource::Default,
        bound: false,
        ignored_override: None,
    }
}

fn count(service: &LeastBusy, deployment_id: &str, in_flight: i64) {
    let key = request_count_key("gpt-4o", deployment_id).unwrap();
    service
        .local
        .set(
            &key,
            &RouteValue::Counter(in_flight),
            Some(IN_FLIGHT_COUNT_TTL_SECONDS),
        )
        .unwrap();
}

#[tokio::test]
async fn the_lowest_count_wins() {
    let deployments = group_and_ids(&[("gpt-4o", "a"), ("gpt-4o", "b")]);
    let service = Arc::new(LeastBusy {
        local: in_flight_store(),
        shared: None,
    });
    count(&service, "a", 2);
    count(&service, "b", 1);
    let counted = Counted(Arc::clone(&service));
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &counted, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::LeastBusy, None).unwrap();
    let resolution = resolution();
    let context = strategy_context(&deployments, &route, &mut dispatch, &resolution);
    let mut random = StdRng::seed_from_u64(7);
    assert_eq!(select(&context, &mut random).await.unwrap(), 1);
}

#[tokio::test]
async fn a_router_without_counters_answers_the_first_deployment() {
    let deployments = group_and_ids(&[("gpt-4o", "a"), ("gpt-4o", "b")]);
    let services = NoServices;
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::LeastBusy, None).unwrap();
    let resolution = resolution();
    let context = strategy_context(&deployments, &route, &mut dispatch, &resolution);
    let mut random = StdRng::seed_from_u64(7);
    assert_eq!(select(&context, &mut random).await.unwrap(), 0);
}

#[tokio::test]
async fn an_empty_pool_answers_zero() {
    let deployments: [DeploymentSpec; 0] = [];
    let service = Arc::new(LeastBusy {
        local: in_flight_store(),
        shared: None,
    });
    let counted = Counted(service);
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &counted, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::LeastBusy, None).unwrap();
    let resolution = resolution();
    let context = strategy_context(&deployments, &route, &mut dispatch, &resolution);
    let mut random = StdRng::seed_from_u64(7);
    assert_eq!(select(&context, &mut random).await.unwrap(), 0);
}
