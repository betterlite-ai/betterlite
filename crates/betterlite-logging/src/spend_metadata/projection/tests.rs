#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::error::Category;

use super::{SpendMetadataArguments, project_spend_logs_metadata};
use crate::spend_metadata::records::{SpendLogsMetadata, SpendLogsRouterMetadata};
use crate::spend_sanitize::SpendSanitizePolicy;

const SK_ABC: &str = "1460db1b6902f8b1fc2a40d9381a24d0fd22c3bc1b2c6f999c521da73776fbe0";
const SK_ABC_HASHED: &str = "125608c42b7910775fab830f88ae315427bfbb945e21af384f19f28d77918bfe";

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture is a JSON object")
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

fn policy(store_bodies: bool) -> SpendSanitizePolicy {
    SpendSanitizePolicy {
        store_bodies,
        max_string_length: 1_000,
    }
}

fn project(
    metadata: Option<&JsonObject>,
    arguments: SpendMetadataArguments,
    store_bodies: bool,
) -> SpendLogsMetadata {
    project_spend_logs_metadata(metadata, arguments, &policy(store_bodies))
        .expect("the metadata projects")
}

#[test]
fn the_absent_mapping_builds_the_constructor_payload() {
    let arguments = enriched_arguments();

    let value = serde_json::to_value(project(None, arguments, true)).unwrap();
    let fields = value.as_object().expect("the record is an object");
    assert_eq!(fields.len(), 42);
    assert_eq!(value["status"], "success");
    assert_eq!(value["cold_storage_object_key"], "cold");
    assert_eq!(value["litellm_call_id"], "call");
    assert_eq!(value["autorouter_savings"], 0.75);
    assert_eq!(value["autorouter_savings_estimate"]["saved"], 2);
    assert_eq!(value["autorouter_baseline_observation"], "baseline");
    assert_eq!(value["router_metadata"]["requested_model"], "requested");
    assert_eq!(
        value["router_metadata"]["selected_model"],
        serde_json::json!(null)
    );
    assert_eq!(value["azure_spillover"]["region"], "west");
    assert_eq!(value["used_client_oauth_token"], false);
    assert_eq!(value["applied_guardrails"], serde_json::json!(null));
    assert!(!fields.contains_key("actor_agent_id"));
    assert!(!fields.contains_key("routing_decision"));
}

#[rstest]
#[case::actor_agent_id("actor_agent_id")]
#[case::target_agent_id("target_agent_id")]
#[case::billing_agent_id("billing_agent_id")]
#[case::agent_execution_mode("agent_execution_mode")]
#[case::verified_human_user_id("verified_human_user_id")]
#[case::routing_decision("routing_decision")]
#[case::user_api_key("user_api_key")]
#[case::status("status")]
fn an_empty_mapping_has_every_field(#[case] name: &str) {
    let metadata = object("{}");
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    let fields = value.as_object().expect("the record is an object");
    assert_eq!(fields.len(), 48);
    assert_eq!(value[name], serde_json::json!(null), "{name} is null");
}

#[rstest]
#[case::router_metadata("router_metadata")]
#[case::azure_spillover("azure_spillover")]
#[case::autorouter_savings("autorouter_savings")]
#[case::autorouter_savings_estimate("autorouter_savings_estimate")]
#[case::autorouter_baseline_observation("autorouter_baseline_observation")]
#[case::used_client_oauth_token("used_client_oauth_token")]
#[case::applied_guardrails("applied_guardrails")]
#[case::batch_models("batch_models")]
#[case::batch_successful_requests("batch_successful_requests")]
#[case::batch_failed_requests("batch_failed_requests")]
#[case::mcp_tool_call_metadata("mcp_tool_call_metadata")]
#[case::vector_store_request_metadata("vector_store_request_metadata")]
#[case::guardrail_information("guardrail_information")]
#[case::usage_object("usage_object")]
#[case::model_map_information("model_map_information")]
#[case::cold_storage_object_key("cold_storage_object_key")]
#[case::litellm_overhead_time_ms("litellm_overhead_time_ms")]
#[case::cost_breakdown("cost_breakdown")]
#[case::litellm_call_id("litellm_call_id")]
fn forged_stamped_values_never_win(#[case] name: &str) {
    let metadata = object(
        r#"{
            "router_metadata": {"requested_model": "forged"},
            "azure_spillover": {"region": "forged"},
            "autorouter_savings": 99.0,
            "autorouter_savings_estimate": {"saved": 99},
            "autorouter_baseline_observation": "forged",
            "used_client_oauth_token": true,
            "applied_guardrails": ["forged"],
            "batch_models": ["forged"],
            "batch_successful_requests": 9,
            "batch_failed_requests": 9,
            "mcp_tool_call_metadata": {"tool": "forged"},
            "vector_store_request_metadata": [{"vector_store_id": "forged"}],
            "guardrail_information": [{"guardrail_name": "forged"}],
            "usage_object": {"total_tokens": 9},
            "model_map_information": {"model": "forged"},
            "cold_storage_object_key": "forged",
            "litellm_overhead_time_ms": 9.5,
            "cost_breakdown": {"total_cost": 9},
            "litellm_call_id": "forged"
        }"#,
    );
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value[name], serde_json::json!(null), "{name} is null");
}

fn enriched_arguments() -> SpendMetadataArguments {
    SpendMetadataArguments {
        applied_guardrails: Some(vec!["guard".to_owned()]),
        batch_models: Some(vec!["batch".to_owned()]),
        batch_successful_requests: Some(3),
        batch_failed_requests: Some(1),
        mcp_tool_call_metadata: Some(object(r#"{"tool":"mcp"}"#)),
        vector_store_request_metadata: Some(vec![json(r#"{"vector_store_id":"vs"}"#)]),
        guardrail_information: Some(json(r#"{"guardrail_name":"guard"}"#)),
        usage_object: Some(object(r#"{"total_tokens":4}"#)),
        model_map_information: Some(object(r#"{"model":"mapped"}"#)),
        cold_storage_object_key: Some("cold".to_owned()),
        litellm_overhead_time_ms: Some(Decimal::new(225, 2)),
        cost_breakdown: Some(serde_json::from_str(r#"{"total_cost":0.5}"#).unwrap()),
        litellm_call_id: Some("call".to_owned()),
        autorouter_savings: Some(Decimal::new(75, 2)),
        autorouter_savings_estimate: Some(object(r#"{"saved":2}"#)),
        baseline_observation: Some("baseline".to_owned()),
        router_metadata: Some(SpendLogsRouterMetadata {
            requested_model: Some("requested".to_owned()),
            selected_model: None,
            selected_provider: Some("openai".to_owned()),
            router_correlation_id: None,
        }),
        azure_spillover: Some(object(r#"{"region":"west"}"#)),
        used_client_oauth_token: Some(false),
    }
}

#[test]
fn the_arguments_are_projected_into_the_record() {
    let arguments = enriched_arguments();

    let metadata = object("{}");
    let value = serde_json::to_value(project(Some(&metadata), arguments, true)).unwrap();
    assert_eq!(value["applied_guardrails"][0], "guard");
    assert_eq!(value["batch_models"][0], "batch");
    assert_eq!(value["batch_successful_requests"], 3);
    assert_eq!(value["batch_failed_requests"], 1);
    assert_eq!(value["mcp_tool_call_metadata"]["tool"], "mcp");
    assert_eq!(
        value["vector_store_request_metadata"][0]["vector_store_id"],
        "vs"
    );
    assert_eq!(value["guardrail_information"]["guardrail_name"], "guard");
    assert_eq!(value["usage_object"]["total_tokens"], 4);
    assert_eq!(value["model_map_information"]["model"], "mapped");
}

#[test]
fn trusted_arguments_are_projected() {
    let metadata = object("{}");
    let value = serde_json::to_value(project(Some(&metadata), enriched_arguments(), true)).unwrap();
    assert_eq!(value["cold_storage_object_key"], "cold");
    assert_eq!(value["litellm_overhead_time_ms"], 2.25);
    assert_eq!(value["cost_breakdown"]["total_cost"], 0.5);
    assert_eq!(value["litellm_call_id"], "call");
    assert_eq!(value["autorouter_savings"], 0.75);
    assert_eq!(value["autorouter_savings_estimate"]["saved"], 2);
    assert_eq!(value["autorouter_baseline_observation"], "baseline");
    assert_eq!(value["router_metadata"]["requested_model"], "requested");
    assert_eq!(value["azure_spillover"]["region"], "west");
    assert_eq!(value["used_client_oauth_token"], false);
}

#[test]
fn copied_metadata_fields_keep_their_values() {
    let metadata = object(
        r#"{
            "status": "failure",
            "user_agent": "ua",
            "attempted_retries": 2,
            "max_retries": 3,
            "additional_usage_values": {"cache_hits": 1},
            "actor_agent_id": "actor",
            "routing_decision": {"model": "chosen"}
        }"#,
    );
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value["status"], "failure");
    assert_eq!(value["user_agent"], "ua");
    assert_eq!(value["attempted_retries"], 2);
    assert_eq!(value["max_retries"], 3);
    assert_eq!(value["additional_usage_values"]["cache_hits"], 1);
    assert_eq!(value["actor_agent_id"], "actor");
    assert_eq!(value["routing_decision"]["model"], "chosen");
}

#[test]
fn a_mistyped_copied_field_names_the_record_and_the_field() {
    let metadata = object(r#"{"status": 5}"#);
    let error = project_spend_logs_metadata(
        Some(&metadata),
        SpendMetadataArguments::default(),
        &policy(true),
    )
    .expect_err("the status is not text");
    let message = error.to_string();
    assert!(message.contains("spend log metadata"), "{message}");
    assert!(message.contains("`status`"), "{message}");
}

#[test]
fn a_raw_secret_is_hashed() {
    let metadata = object(r#"{"user_api_key": "sk-abc"}"#);
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value["user_api_key"], SK_ABC);
}

#[test]
fn a_bearer_prefixed_secret_is_hashed_without_the_prefix() {
    let metadata = object(r#"{"user_api_key": "Bearer sk-abc"}"#);
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value["user_api_key"], SK_ABC);
}

#[test]
fn a_trusted_digest_is_kept() {
    let metadata = object(&format!(
        r#"{{"user_api_key": "{SK_ABC}", "user_api_key_hash": "{SK_ABC}"}}"#
    ));
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value["user_api_key"], SK_ABC);
}

#[test]
fn an_untrusted_digest_is_hashed_again() {
    let metadata = object(&format!(r#"{{"user_api_key": "{SK_ABC}"}}"#));
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value["user_api_key"], SK_ABC_HASHED);
}

#[test]
fn a_trusted_hash_that_differs_from_the_key_does_not_disable_hashing() {
    let metadata = object(&format!(
        r#"{{"user_api_key": "{SK_ABC}", "user_api_key_hash": "litellm-cli"}}"#
    ));
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value["user_api_key"], SK_ABC_HASHED);
}

#[test]
fn a_non_text_trusted_hash_does_not_disable_hashing() {
    let metadata = object(&format!(
        r#"{{"user_api_key": "{SK_ABC}", "user_api_key_hash": 5}}"#
    ));
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value["user_api_key"], SK_ABC_HASHED);
}

#[test]
fn the_alias_allows_a_cli_session_key() {
    let metadata = object(
        r#"{
            "user_api_key": "cli-session-abc",
            "user_api_key_hash": "cli-session-abc",
            "user_api_key_alias": "cli-session-abc"
        }"#,
    );
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value["user_api_key"], "cli-session-abc");
    assert_eq!(value["user_api_key_alias"], "cli-session-abc");
}

#[rstest]
#[case::number(r#"{"user_api_key": 5}"#)]
#[case::empty(r#"{"user_api_key": ""}"#)]
#[case::bare_bearer(r#"{"user_api_key": "Bearer "}"#)]
fn non_text_and_empty_keys_become_null(#[case] fixture: &str) {
    let metadata = object(fixture);
    let value = serde_json::to_value(project(
        Some(&metadata),
        SpendMetadataArguments::default(),
        true,
    ))
    .unwrap();
    assert_eq!(value["user_api_key"], serde_json::json!(null), "{fixture}");
}

#[test]
fn vector_store_bodies_are_redacted_unless_bodies_are_stored() {
    let arguments: SpendMetadataArguments = serde_json::from_str(
        r#"{"vector_store_request_metadata":[{"vector_store_search_response":{"data":[{"content":[{"text":"secret"}]}]}}]}"#,
    ).unwrap();
    let metadata = object("{}");
    let redacted =
        serde_json::to_value(project(Some(&metadata), arguments.clone(), false)).unwrap();
    assert_eq!(
        redacted["vector_store_request_metadata"][0]["vector_store_search_response"]["data"][0]["content"]
            [0]["text"],
        "REDACTED_BY_LITELM"
    );
    let stored = serde_json::to_value(project(Some(&metadata), arguments, true)).unwrap();
    assert_eq!(
        stored["vector_store_request_metadata"][0]["vector_store_search_response"]["data"][0]["content"]
            [0]["text"],
        "secret"
    );
}

#[test]
fn guardrail_information_is_sanitized_unless_bodies_are_stored() {
    let arguments: SpendMetadataArguments = serde_json::from_str(
        r#"{"guardrail_information":[{"guardrail_name":"guard","guardrail_response":"secret"}]}"#,
    )
    .unwrap();
    let metadata = object("{}");
    let redacted =
        serde_json::to_value(project(Some(&metadata), arguments.clone(), false)).unwrap();
    assert_eq!(
        redacted["guardrail_information"][0]["guardrail_response"],
        "REDACTED_BY_LITELM"
    );
    let stored = serde_json::to_value(project(Some(&metadata), arguments, true)).unwrap();
    assert_eq!(
        stored["guardrail_information"][0]["guardrail_response"],
        "secret"
    );
}

#[test]
fn savings_preserve_decimal_precision() {
    let arguments: SpendMetadataArguments =
        serde_json::from_str(r#"{"autorouter_savings":0.1234567890123456789012345678}"#).unwrap();
    let value = serde_json::to_value(project(Some(&object("{}")), arguments, true)).unwrap();
    assert_eq!(
        value["autorouter_savings"].to_string(),
        "0.1234567890123456789012345678"
    );
}

// The helper decodes its arguments inside a tagged call, whose content buffer turns a null into unit.
#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum TaggedCall {
    Project {
        arguments: Box<SpendMetadataArguments>,
    },
}

#[rstest]
#[case::null(r#"{"function":"project","arguments":{"autorouter_savings":null,"litellm_overhead_time_ms":null}}"#)]
#[case::absent(r#"{"function":"project","arguments":{}}"#)]
fn a_tagged_call_reads_an_absent_or_null_decimal_as_default(#[case] fixture: &str) {
    let TaggedCall::Project { arguments } = serde_json::from_str(fixture).unwrap();
    assert_eq!(arguments.autorouter_savings, None);
    assert_eq!(arguments.litellm_overhead_time_ms, None);
}

#[test]
fn cost_breakdown_rejects_non_numeric_amounts() {
    let fixture = r#"{"cost_breakdown":{"total_cost":{"unexpected":0.5}}}"#;
    let arguments_error = serde_json::from_str::<SpendMetadataArguments>(fixture).unwrap_err();
    assert_eq!(arguments_error.classify(), Category::Data);
    let record_error = serde_json::from_str::<SpendLogsMetadata>(fixture).unwrap_err();
    assert_eq!(record_error.classify(), Category::Data);
}
