use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use serde_json::Number;

use super::support::{Fixture, deployment, ids, names, object, text};
use crate::health::router::{
    dependency_deployments_to_probe, deployment_id, resolved_deployment_ids,
};

// The complexity router a fixture names a tier on: the config is what strategy_router_dependencies reads.
const TIER_ROUTER: &str = r#"{"model":"auto_router/complexity_router","complexity_router_config":{"tiers":{"SIMPLE":"%NAME%"}}}"#;

fn tiers(name: &str) -> String {
    TIER_ROUTER.replace("%NAME%", name)
}

#[test]
fn id_extraction() {
    let named = deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#);
    assert_eq!(deployment_id(&named), Some("leaf-id"));
    let empty = deployment("leaf", "", r#"{"model":"openai/leaf"}"#);
    assert_eq!(deployment_id(&empty), None);
    let numeric = DeploymentSpec::new(
        "leaf".to_owned(),
        object(vec![("model", text("openai/leaf"))]),
        object(vec![("id", Json::Number(Number::from(7_u64)))]),
        JsonObject::new(),
        String::new,
    )
    .unwrap();
    assert_eq!(deployment_id(&numeric), Some("7"));
    let unindexed = DeploymentSpec {
        model_name: "leaf".to_owned(),
        litellm_params: object(vec![("model", text("openai/leaf"))]),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    };
    assert_eq!(deployment_id(&unindexed), None);
}

#[test]
fn name_resolution() {
    let rows = vec![
        deployment("multi", "multi-a", r#"{"model":"openai/multi"}"#),
        deployment("multi", "multi-b", r#"{"model":"openai/multi"}"#),
        deployment("twin", "twin-id", r#"{"model":"openai/twin"}"#),
        deployment("twin", "twin-id", r#"{"model":"openai/twin"}"#),
        deployment("real", "real-id", r#"{"model":"openai/real"}"#),
        deployment("blank", "", r#"{"model":"openai/blank"}"#),
    ];
    let fixture = Fixture::with_aliases(&rows, &[("alias", "real"), ("gone", "vanished")]);
    let lookup = fixture.lookup();
    assert_eq!(
        resolved_deployment_ids(&lookup, "multi").unwrap(),
        Some(ids(&["multi-a", "multi-b"]))
    );
    assert_eq!(
        resolved_deployment_ids(&lookup, "twin").unwrap(),
        Some(ids(&["twin-id"]))
    );
    assert_eq!(
        resolved_deployment_ids(&lookup, "alias").unwrap(),
        Some(ids(&["real-id"]))
    );
    assert_eq!(resolved_deployment_ids(&lookup, "gone").unwrap(), None);
    assert_eq!(resolved_deployment_ids(&lookup, "unknown").unwrap(), None);
    // a name whose deployments carry no usable id resolves to an empty set, not to nothing
    assert_eq!(
        resolved_deployment_ids(&lookup, "blank").unwrap(),
        Some(ids(&[]))
    );
}

#[test]
fn expansion_dedup() {
    let rows = vec![
        deployment("outer", "outer-id", &tiers("mid")),
        deployment(
            "mid",
            "mid-id",
            r#"{"model":"auto_router/adaptive_router","adaptive_router_config":{"available_models":["leaf"]}}"#,
        ),
        deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#),
        deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#),
        deployment("ignored", "ignored-id", r#"{"model":"openai/ignored"}"#),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    let expanded = dependency_deployments_to_probe(&rows[..1], &rows, &lookup).unwrap();
    // the chain is walked through the child router, the duplicate id is probed once, and the requested router is not reported back
    assert_eq!(names(&expanded), ["mid", "leaf"]);
    let expanded_ids: Vec<&str> = expanded.iter().filter_map(|row| row.id()).collect();
    assert_eq!(ids(&expanded_ids), ids(&["mid-id", "leaf-id"]));
}

#[test]
fn expansion_skips_evaluation_dependencies() {
    let rows = vec![
        deployment(
            "outer",
            "outer-id",
            r#"{"model":"auto_router/complexity_router","complexity_router_config":{"classifier_type":"jev"}}"#,
        ),
        deployment(
            "typesafe/jev-latest",
            "jev-id",
            r#"{"model":"typesafe/jev-latest"}"#,
        ),
    ];
    let fixture = Fixture::new(&rows);
    let lookup = fixture.lookup();
    assert!(
        dependency_deployments_to_probe(&rows[..1], &rows, &lookup)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn expansion_cycle() {
    let chain = vec![
        deployment("a", "a-id", &tiers("b")),
        deployment("b", "b-id", &tiers("a")),
    ];
    let chain_fixture = Fixture::new(&chain);
    let chain_lookup = chain_fixture.lookup();
    let expanded = dependency_deployments_to_probe(&chain[..1], &chain, &chain_lookup).unwrap();
    assert_eq!(names(&expanded), ["b"]);

    let solo = vec![deployment("solo", "solo-id", &tiers("solo"))];
    let solo_fixture = Fixture::new(&solo);
    let solo_lookup = solo_fixture.lookup();
    assert!(
        dependency_deployments_to_probe(&solo, &solo, &solo_lookup)
            .unwrap()
            .is_empty()
    );
}
