#![cfg(test)]

use crate::router_core::candidates::CandidateSet;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request,
};
use crate::static_eligibility::request::RequestFacts;

use super::apply;

#[tokio::test]
async fn router_core_boundaries() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let error = apply(CandidateSet::default(), &route).await.unwrap_err();
    assert_eq!(error.unbuilt_entry(), Some("generalization_integration"));
}
