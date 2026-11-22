#![cfg(test)]

use betterlite_prompts::arguments::json::Object;

use crate::affinity_claims::identity::AffinityFlag;
use crate::deployment_affinity::DeploymentAffinity;
use crate::deployment_affinity::tests::support::{
    Fixture, MODEL_GROUP, grouped, pin, pool, settings, user_pin,
};
use crate::router_core::candidates::CandidateSet;
use crate::router_core::services::RouteServices;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request,
};
use crate::static_eligibility::request::RequestFacts;

use super::apply;

// the check the server registers when the config turns affinity on
struct Pinned<'fixture> {
    affinity: &'fixture DeploymentAffinity,
}

impl RouteServices for Pinned<'_> {
    fn deployment_affinity(&self) -> Option<&DeploymentAffinity> {
        Some(self.affinity)
    }
}

fn caller() -> Object {
    serde_json::from_str(r#"{"metadata": {"user_api_key_hash": "key"}}"#).unwrap()
}

async fn selected(
    services: &dyn RouteServices,
    model: &str,
    target_order: Option<i64>,
    kwargs: Option<&Object>,
    candidates: CandidateSet,
) -> Vec<String> {
    let facts = RequestFacts {
        target_order,
        ..RequestFacts::default()
    };
    let request = route_request(model, &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let mut route = route_context(&request, services, &limits, &params);
    route.policy.prompt.request_kwargs = kwargs;
    let kept = apply(candidates, &route).await.unwrap();
    kept.deployments
        .iter()
        .filter_map(|deployment| deployment.id().map(str::to_owned))
        .collect()
}

fn candidates() -> CandidateSet {
    CandidateSet {
        deployments: pool(&["a", "b"]),
    }
}

// a router that does not register the check leaves the pool to the pre-call checks and the strategy
#[tokio::test]
async fn a_router_without_the_check_keeps_the_pool() {
    let kwargs = caller();
    let selected = selected(&NoServices, MODEL_GROUP, None, Some(&kwargs), candidates()).await;
    assert_eq!(selected, ["a", "b"]);
}

// the caller key's pin narrows the pool before the strategy picks
#[tokio::test]
async fn the_check_narrows_the_pool_to_the_pinned_deployment() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let services = Pinned {
        affinity: &fixture.service,
    };
    let kwargs = caller();
    let selected = selected(&services, MODEL_GROUP, None, Some(&kwargs), candidates()).await;
    assert_eq!(selected, ["a"]);
}

// the group whose flags apply is the pool's model group, which LiteLLM resolved from the alias before the callbacks ran
#[tokio::test]
async fn the_pool_names_the_check_not_the_requested_alias() {
    let fixture = grouped(&[], &[(MODEL_GROUP, &["deployment_affinity"])]);
    fixture.seed(&user_pin("key"), &pin("b"), 600);
    let services = Pinned {
        affinity: &fixture.service,
    };
    let kwargs = caller();
    let selected = selected(&services, "alias", None, Some(&kwargs), candidates()).await;
    assert_eq!(selected, ["b"]);
}

// a fallback retry carries _target_order, which stands the check down and keeps the pool whole
#[tokio::test]
async fn a_target_order_keeps_the_pool() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let services = Pinned {
        affinity: &fixture.service,
    };
    let kwargs = caller();
    let selected = selected(&services, MODEL_GROUP, Some(2), Some(&kwargs), candidates()).await;
    assert_eq!(selected, ["a", "b"]);
}

// a request that carries no arguments yet has no caller or session identity to pin
#[tokio::test]
async fn a_request_without_arguments_keeps_the_pool() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    fixture.seed(&user_pin("key"), &pin("a"), 600);
    let services = Pinned {
        affinity: &fixture.service,
    };
    let selected = selected(&services, MODEL_GROUP, None, None, candidates()).await;
    assert_eq!(selected, ["a", "b"]);
}

// an empty pool has no model group to name the check, and stays empty
#[tokio::test]
async fn an_empty_pool_stays_empty() {
    let fixture = Fixture::new(settings(&[AffinityFlag::Deployment]), &[]);
    let services = Pinned {
        affinity: &fixture.service,
    };
    let kwargs = caller();
    let selected = selected(
        &services,
        MODEL_GROUP,
        None,
        Some(&kwargs),
        CandidateSet::default(),
    )
    .await;
    assert!(selected.is_empty());
}
