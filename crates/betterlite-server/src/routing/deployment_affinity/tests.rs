#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::deployment_affinity::Settings as SectionSettings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::identity::AffinityFlag;
use betterlite_router::affinity_claims::local::LocalPins;
use betterlite_router::deployment_affinity::{FilterRequest, PriorityGroups};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::routing_value_store::{Clock, StoreSettings};
use indexmap::IndexMap;

use super::{AuthenticatedRequest, DeploymentAffinityInputs, build, record_selection};

fn section(checks: Option<&[&str]>, groups: Option<&[(&str, &[&str])]>) -> SectionSettings {
    let optional = checks.map(|names| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<String>>()
    });
    let config = groups.map(|entries| {
        let mut map = IndexMap::with_capacity(entries.len());
        for (group, flags) in entries {
            map.insert(
                (*group).to_owned(),
                flags.iter().map(|flag| (*flag).to_owned()).collect(),
            );
        }
        map
    });
    SectionSettings {
        optional_pre_call_checks: optional,
        model_group_affinity_config: config,
        ttl_seconds: 900,
    }
}

fn claims() -> AffinityClaims {
    let clock: Clock = Arc::new(|| 0.0_f64);
    let local = LocalPins::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: Some(1024),
            loaded_max_size_per_item_kib: 1024,
        },
        clock,
    )
    .unwrap();
    AffinityClaims {
        local,
        shared: None,
    }
}

fn inputs(groups: &[&str]) -> DeploymentAffinityInputs {
    let names: Vec<String> = groups.iter().map(|group| (*group).to_owned()).collect();
    let predicate: PriorityGroups =
        Arc::new(move |model: &str| names.iter().any(|group| group == model));
    DeploymentAffinityInputs {
        claims: claims(),
        is_priority_group: Some(predicate),
        session_marker_in_use: false,
    }
}

fn deployment(model: &str, id: &str) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(format!("openai/{id}")));
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(id.to_owned()));
    DeploymentSpec::new(
        model.to_owned(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

// without a named check or a per-group entry the router registers no affinity callback
#[test]
fn no_flag_and_no_config_builds_nothing() {
    assert!(build(&section(None, None), inputs(&[])).is_none());
    assert!(build(&section(Some(&["prompt_caching"]), Some(&[])), inputs(&[])).is_none());
}

// a named check enables its flag for every group
#[test]
fn a_named_check_becomes_a_global_flag() {
    let affinity = build(&section(Some(&["session_affinity"]), None), inputs(&[])).unwrap();
    let flags = affinity.flags_for("gpt-4o");
    assert!(flags.contains(&AffinityFlag::Session));
    assert!(!flags.contains(&AffinityFlag::Deployment));
}

// a complexity router that stamps the session marker registers the callback with every global flag off
#[test]
fn the_session_marker_alone_builds_the_check() {
    let affinity = build(
        &section(None, None),
        DeploymentAffinityInputs {
            claims: claims(),
            is_priority_group: None,
            session_marker_in_use: true,
        },
    )
    .unwrap();
    assert!(affinity.flags_for("gpt-4o").is_empty());
}

// the per-group config alone registers the callback with every global flag off
#[test]
fn the_per_group_config_alone_builds_the_check() {
    let groups: &[(&str, &[&str])] = &[("gpt-4o", &["deployment_affinity"])];
    let affinity = build(&section(Some(&[]), Some(groups)), inputs(&[])).unwrap();
    assert!(
        affinity
            .flags_for("gpt-4o")
            .contains(&AffinityFlag::Deployment)
    );
    assert!(affinity.flags_for("other").is_empty());
}

// the pre-call hook reads the auth metadata and the selected model_info the server attaches
#[tokio::test]
async fn the_authenticated_context_pins_the_selected_deployment() {
    let affinity = build(&section(Some(&["deployment_affinity"]), None), inputs(&[])).unwrap();
    let metadata = Json::Object(IndexMap::from([
        (
            "deployment_model_name".to_owned(),
            Json::Text("gpt-4o".to_owned()),
        ),
        ("user_api_key_hash".to_owned(), Json::Text("key".to_owned())),
    ]));
    let model_info = Json::Object(IndexMap::from([(
        "id".to_owned(),
        Json::Text("dep-a".to_owned()),
    )]));
    record_selection(
        &affinity,
        &AuthenticatedRequest {
            metadata: Some(&metadata),
            litellm_metadata: None,
            model_info: Some(&model_info),
        },
    )
    .await;
    let pool = vec![deployment("gpt-4o", "dep-a"), deployment("gpt-4o", "dep-b")];
    let request_kwargs = IndexMap::from([("metadata".to_owned(), metadata)]);
    let selected = affinity
        .filter_deployments(&FilterRequest {
            model: "gpt-4o",
            deployments: &pool,
            request_kwargs: Some(&request_kwargs),
            target_order: None,
        })
        .await
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id(), Some("dep-a"));
}

// a request with unusable metadata fails neither the hook nor the response
#[tokio::test]
async fn an_unusable_request_keeps_the_response() {
    let affinity = build(&section(Some(&["deployment_affinity"]), None), inputs(&[])).unwrap();
    let metadata = Json::Object(IndexMap::from([
        (
            "deployment_model_name".to_owned(),
            Json::Text("gpt-4o".to_owned()),
        ),
        ("user_api_key_hash".to_owned(), Json::Bool(true)),
    ]));
    let model_info = Json::Object(IndexMap::from([(
        "id".to_owned(),
        Json::Text("dep-a".to_owned()),
    )]));
    record_selection(
        &affinity,
        &AuthenticatedRequest {
            metadata: Some(&metadata),
            litellm_metadata: None,
            model_info: Some(&model_info),
        },
    )
    .await;
    let pool = vec![deployment("gpt-4o", "dep-a"), deployment("gpt-4o", "dep-b")];
    let request_kwargs = IndexMap::from([(
        "metadata".to_owned(),
        Json::Object(IndexMap::from([(
            "user_api_key_hash".to_owned(),
            Json::Text("key".to_owned()),
        )])),
    )]);
    let selected = affinity
        .filter_deployments(&FilterRequest {
            model: "gpt-4o",
            deployments: &pool,
            request_kwargs: Some(&request_kwargs),
            target_order: None,
        })
        .await
        .unwrap();
    let ids: Vec<Option<&str>> = selected.iter().map(DeploymentSpec::id).collect();
    assert_eq!(ids, [Some("dep-a"), Some("dep-b")]);
}
