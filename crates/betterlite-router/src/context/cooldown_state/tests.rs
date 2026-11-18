#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::cooldown_state::filter::HealthInputs;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::services::RouteServices;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request,
};
use crate::static_eligibility::request::RequestFacts;

use super::apply;

struct Cooling(Vec<String>);

impl RouteServices for Cooling {
    fn cooldown_list(&self) -> Vec<String> {
        self.0.clone()
    }
}

struct Health(HealthInputs);

impl RouteServices for Health {
    fn health_inputs(&self) -> HealthInputs {
        self.0.clone()
    }
}

struct Both {
    cooling: Vec<String>,
    health: HealthInputs,
}

impl RouteServices for Both {
    fn cooldown_list(&self) -> Vec<String> {
        self.cooling.clone()
    }

    fn health_inputs(&self) -> HealthInputs {
        self.health.clone()
    }
}

fn unhealthy(ids: &[&str]) -> HealthInputs {
    HealthInputs {
        enabled: true,
        has_allowed_fails_policy: false,
        scoped_groups: None,
        unhealthy: ids.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn deployment(model_name: &str, id: &str) -> DeploymentSpec {
    let mut litellm = JsonObject::new();
    litellm.insert("model".to_owned(), Json::Text(format!("openai/{id}")));
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new(
        model_name.to_owned(),
        litellm,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn kept(result: &CandidateSet) -> Vec<String> {
    result
        .deployments
        .iter()
        .filter_map(DeploymentSpec::id)
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn no_cooling_deployment_keeps_the_pool() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("gpt-4o", "a"), deployment("gpt-4o", "b")],
    };
    let result = apply(set.clone(), &route).await.unwrap();
    assert_eq!(result, set);
}

#[tokio::test]
async fn a_cooling_deployment_leaves_the_pool() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let services = Cooling(vec![String::from("a")]);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("gpt-4o", "a"), deployment("gpt-4o", "b")],
    };
    let result = apply(set, &route).await.unwrap();
    assert_eq!(kept(&result), vec![String::from("b")]);
}

#[tokio::test]
async fn an_unhealthy_deployment_leaves_the_pool() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let services = Health(unhealthy(&["a"]));
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("gpt-4o", "a"), deployment("gpt-4o", "b")],
    };
    let result = apply(set, &route).await.unwrap();
    assert_eq!(kept(&result), vec![String::from("b")]);
}

#[tokio::test]
async fn the_health_filter_runs_before_the_cooldown_filter() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let services = Both {
        cooling: vec![String::from("b")],
        health: unhealthy(&["a"]),
    };
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("gpt-4o", "a"), deployment("gpt-4o", "b")],
    };
    let result = apply(set, &route).await.unwrap();
    assert!(result.deployments.is_empty());
}

#[tokio::test]
async fn an_allowed_fails_policy_keeps_unhealthy_deployments() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let mut inputs = unhealthy(&["a"]);
    inputs.has_allowed_fails_policy = true;
    let services = Health(inputs);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let set = CandidateSet {
        deployments: vec![deployment("gpt-4o", "a"), deployment("gpt-4o", "b")],
    };
    let result = apply(set.clone(), &route).await.unwrap();
    assert_eq!(result, set);
}
