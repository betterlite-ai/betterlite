#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

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
