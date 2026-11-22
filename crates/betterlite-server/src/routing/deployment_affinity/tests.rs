#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::deployment_affinity::Settings as SectionSettings;
use betterlite_config::types::Config;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::arguments::json::Object;
use betterlite_router::affinity_claims::AffinityClaims;
use betterlite_router::affinity_claims::identity::AffinityFlag;
use betterlite_router::affinity_claims::local::LocalPins;
use betterlite_router::deployment_affinity::{FilterRequest, PriorityGroups};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::routing_value_store::{Clock, StoreSettings};
use indexmap::IndexMap;

use super::{
    DeploymentAffinityInputs, build, from_config, priority_groups, record_selection, stamped_kwargs,
};

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

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
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

// the pre-call hook reads the auth metadata and the deployment stamps the route attached
#[tokio::test]
async fn the_authenticated_context_pins_the_selected_deployment() {
    let affinity = build(&section(Some(&["deployment_affinity"]), None), inputs(&[])).unwrap();
    let chosen = deployment("gpt-4o", "dep-a");
    let kwargs = caller();
    record_selection(&affinity, &chosen, Some(&kwargs)).await;
    let pool = vec![chosen, deployment("gpt-4o", "dep-b")];
    let selected = affinity
        .filter_deployments(&FilterRequest {
            model: "gpt-4o",
            deployments: &pool,
            request_kwargs: Some(&caller_kwargs()),
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
    let kwargs: Object =
        serde_json::from_str(r#"{"metadata": {"user_api_key_hash": true}}"#).unwrap();
    record_selection(&affinity, &deployment("gpt-4o", "dep-a"), Some(&kwargs)).await;
    let pool = vec![deployment("gpt-4o", "dep-a"), deployment("gpt-4o", "dep-b")];
    let selected = affinity
        .filter_deployments(&FilterRequest {
            model: "gpt-4o",
            deployments: &pool,
            request_kwargs: Some(&caller_kwargs()),
            target_order: None,
        })
        .await
        .unwrap();
    let ids: Vec<Option<&str>> = selected.iter().map(DeploymentSpec::id).collect();
    assert_eq!(ids, [Some("dep-a"), Some("dep-b")]);
}

// add_optional_pre_call_checks: the router config names the check, and a config naming nothing registers none
#[test]
fn the_router_config_registers_the_named_check() {
    let named = "model_list: []\nrouter_settings:\n  optional_pre_call_checks:\n    - deployment_affinity\n";
    assert!(from_config(&config(named)).is_some());
    assert!(from_config(&config("model_list: []\n")).is_none());
}

// _ensure_deployment_affinity_callback: a complexity router that stamps the marker registers the check with every global flag off
#[test]
fn a_marker_stamping_router_registers_the_check() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config:\n        deployment_affinity: true\n";
    let affinity = from_config(&config(source)).expect("the marker registers the check");
    assert!(affinity.flags_for("gpt-4o").is_empty());
}

// plugins replace the pin machinery, so a plugin-carrying router registers nothing
#[test]
fn a_plugin_carrying_router_registers_nothing() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config:\n        deployment_affinity: true\n        plugins:\n          - name: p\n";
    assert!(from_config(&config(source)).is_none());
}

// Router._is_priority_routing_group reads the config's routing groups
#[test]
fn the_routing_groups_decide_the_priority_bypass() {
    let source = "model_list: []\nrouter_settings:\n  routing_groups:\n    - group_name: prio\n      models: [gpt-4o]\n      routing_strategy: priority\n      model_priorities: {gpt-4o: 1}\n";
    let is_priority = priority_groups(&config(source)).expect("the groups build");
    assert!(is_priority("prio"));
    assert!(!is_priority("gpt-4o"));
}

// a group index that cannot be built leaves the check without a priority bypass instead of failing the server
#[test]
fn unreadable_routing_groups_leave_no_priority_bypass() {
    let source = "model_list: []\nrouter_settings:\n  routing_groups:\n    - group_name: prio\n      models: [a]\n      routing_strategy: priority\n      model_priorities: {a: 1}\n    - group_name: prio\n      models: [b]\n      routing_strategy: simple-shuffle\n";
    assert!(priority_groups(&config(source)).is_none());
}

// the selection stamps the deployment name and model_info the hook reads, beside the caller's metadata
#[tokio::test]
async fn the_selection_stamps_the_deployment_for_the_hook() {
    let affinity = build(&section(Some(&["deployment_affinity"]), None), inputs(&[])).unwrap();
    let chosen = deployment("gpt-4o", "dep-a");
    let kwargs = caller();
    let stamped = stamped_kwargs(Some(&kwargs), &chosen);
    let Some(Json::Object(metadata)) = stamped.get("metadata") else {
        panic!("the selection stamps a metadata object");
    };
    assert_eq!(
        metadata.get("deployment_model_name"),
        Some(&Json::Text("gpt-4o".to_owned()))
    );
    assert_eq!(
        metadata.get("model_info"),
        Some(&Json::Object(chosen.model_info.clone()))
    );
    assert_eq!(
        metadata.get("user_api_key_hash"),
        Some(&Json::Text("key".to_owned()))
    );
    record_selection(&affinity, &chosen, Some(&kwargs)).await;
    let pool = vec![chosen, deployment("gpt-4o", "dep-b")];
    let selected = affinity
        .filter_deployments(&FilterRequest {
            model: "gpt-4o",
            deployments: &pool,
            request_kwargs: Some(&caller_kwargs()),
            target_order: None,
        })
        .await
        .unwrap();
    assert_eq!(selected[0].id(), Some("dep-a"));
}

// the hook reads the caller from either channel, and the stamp lands in the metadata channel
#[tokio::test]
async fn the_selection_reads_the_litellm_metadata_channel() {
    let affinity = build(&section(Some(&["deployment_affinity"]), None), inputs(&[])).unwrap();
    let kwargs: Object =
        serde_json::from_str(r#"{"litellm_metadata": {"user_api_key_hash": "key"}}"#).unwrap();
    let chosen = deployment("gpt-4o", "dep-a");
    record_selection(&affinity, &chosen, Some(&kwargs)).await;
    let pool = vec![chosen, deployment("gpt-4o", "dep-b")];
    let selected = affinity
        .filter_deployments(&FilterRequest {
            model: "gpt-4o",
            deployments: &pool,
            request_kwargs: Some(&caller_kwargs()),
            target_order: None,
        })
        .await
        .unwrap();
    assert_eq!(selected[0].id(), Some("dep-a"));
}

// a request with no arguments carries no identity, so the stamped metadata claims nothing
#[tokio::test]
async fn a_selection_without_arguments_claims_nothing() {
    let affinity = build(&section(Some(&["deployment_affinity"]), None), inputs(&[])).unwrap();
    let chosen = deployment("gpt-4o", "dep-a");
    record_selection(&affinity, &chosen, None).await;
    let pool = vec![chosen, deployment("gpt-4o", "dep-b")];
    let selected = affinity
        .filter_deployments(&FilterRequest {
            model: "gpt-4o",
            deployments: &pool,
            request_kwargs: Some(&caller_kwargs()),
            target_order: None,
        })
        .await
        .unwrap();
    assert_eq!(selected.len(), 2);
}

fn caller() -> Object {
    serde_json::from_str(r#"{"metadata": {"user_api_key_hash": "key"}}"#).unwrap()
}

fn caller_kwargs() -> JsonObject {
    IndexMap::from([(
        "metadata".to_owned(),
        Json::Object(IndexMap::from([(
            "user_api_key_hash".to_owned(),
            Json::Text("key".to_owned()),
        )])),
    )])
}
