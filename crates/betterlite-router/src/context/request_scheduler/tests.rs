#![cfg(test)]

use crate::context::{RouteStage, STAGE_FLAGS, run};
use crate::router_core::candidates::CandidateSet;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request,
};
use crate::static_eligibility::request::RequestFacts;

// the stage is configured and hands the pool through: the scheduler holds requests, it does not drop deployments
#[tokio::test]
async fn the_stage_hands_its_candidates_through() {
    let configured = STAGE_FLAGS
        .iter()
        .any(|(stage, flag)| *stage == RouteStage::RequestScheduler && *flag);
    assert!(configured);
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let returned = run(
        RouteStage::RequestScheduler,
        CandidateSet::default(),
        &route,
    )
    .await
    .expect("the scheduler stage passes the pool through");
    assert!(returned.deployments.is_empty());
}
