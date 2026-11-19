#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::deployment_catalog::types::Authority;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request,
};
use crate::static_eligibility::request::RequestFacts;

use super::apply;

fn spec(group: &str, id: &str, info: &[(&str, &str)]) -> DeploymentSpec {
    let mut model_info = JsonObject::from_iter([("id".to_owned(), Json::Text(id.to_owned()))]);
    for (key, value) in info {
        model_info.insert((*key).to_owned(), Json::Text((*value).to_owned()));
    }
    DeploymentSpec::new(
        group.to_owned(),
        JsonObject::new(),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    deployments
        .iter()
        .map(|deployment| deployment.id().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn the_stage_keeps_the_callers_team() {
    let facts = RequestFacts::default();
    let mut request = route_request("gpt", &facts);
    request.authority = Authority {
        team_id: Some("t1"),
        proxy_admin: false,
    };
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let candidates = CandidateSet {
        deployments: vec![
            spec("gpt", "shared", &[]),
            spec("gpt", "ours", &[("team_id", "t1")]),
            spec("gpt", "theirs", &[("team_id", "t2")]),
        ],
    };
    let kept = apply(candidates, &route).await.unwrap();
    assert_eq!(ids(&kept.deployments), ["shared", "ours"]);
}

#[tokio::test]
async fn the_stage_refuses_an_ambiguous_admin_pool() {
    let facts = RequestFacts::default();
    let mut request = route_request("shared", &facts);
    request.authority = Authority {
        team_id: None,
        proxy_admin: true,
    };
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let candidates = CandidateSet {
        deployments: vec![
            spec(
                "shared",
                "a",
                &[("team_id", "t1"), ("team_public_model_name", "shared")],
            ),
            spec(
                "shared",
                "b",
                &[("team_id", "t2"), ("team_public_model_name", "shared")],
            ),
        ],
    };
    let error = apply(candidates, &route).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Model name 'shared' matches deployments from multiple teams. Specify the deployment ID directly to disambiguate."
    );
}
