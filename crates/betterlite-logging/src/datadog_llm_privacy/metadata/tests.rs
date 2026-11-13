#![cfg(test)]

use crate::datadog_llm_privacy::tests::support;

use betterlite_models::decisions::Json;

use crate::datadog_llm_obs::values::Object;
use crate::datadog_llm_privacy::metadata::span_metadata;
use crate::datadog_llm_privacy::router_span::router_span_fields;
use crate::fields::payload::StandardLoggingPayload;

const FULL_PAYLOAD: &str = r#"{
  "id": "chatcmpl-1",
  "trace_id": "trace-1",
  "session_id": "session-1",
  "call_type": "completion",
  "response_cost": 0.5,
  "status": "success",
  "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
  "total_tokens": 5,
  "prompt_tokens": 2,
  "completion_tokens": 3,
  "startTime": 1704067200.0,
  "endTime": 1704067201.0,
  "completionStartTime": 1704067200.5,
  "response_time": 1.0,
  "model_map_information": {},
  "model": "gpt-4o",
  "model_group": "gpt-4o",
  "api_base": "https://api.openai.com",
  "saved_cache_cost": 0.0,
  "request_tags": [],
  "model_parameters": {},
  "hidden_params": {},
  "standard_built_in_tools_params": null,
  "metadata": {
    "user_api_key_hash": "hash-1",
    "user_api_key_alias": "prod-key",
    "user_api_key_spend": 1.5,
    "user_api_key_max_budget": 10.25,
    "user_api_key_budget_reset_at": "2026-01-01T00:00:00",
    "user_api_key_user_spend": 0.5,
    "user_api_key_user_max_budget": 5.25,
    "user_api_key_team_spend": 2.5,
    "user_api_key_team_max_budget": 20.25,
    "user_api_key_org_id": "org-1",
    "user_api_key_org_alias": "acme",
    "user_api_key_team_id": "team-1",
    "user_api_key_project_id": "proj-1",
    "user_api_key_project_alias": "demo",
    "user_api_key_user_id": "alice",
    "user_api_key_user_email": "alice@example.com",
    "user_api_key_team_alias": "engineering",
    "user_api_key_end_user_id": "end-1",
    "user_api_key_request_route": "/v1/chat",
    "user_api_key_auth_metadata": {"source": "jwt"},
    "spend_logs_metadata": {"trace": "abc"},
    "requester_ip_address": "203.0.113.5",
    "user_agent": "curl/8",
    "requester_metadata": {"prompt": "secret"},
    "requester_custom_headers": {"x-request-id": "r-1"},
    "prompt_management_metadata": {"prompt_id": "p-1"},
    "mcp_tool_call_metadata": {"tool": "search"},
    "vector_store_request_metadata": [{"id": "vs-1"}],
    "routing_decision": {"tier": "low", "signals": ["quote"], "routed_model": "gpt-4o-mini"},
    "applied_guardrails": ["pii"],
    "usage_object": {"prompt_tokens": 2},
    "cold_storage_object_key": "s3://bucket/key",
    "team_alias": "platform",
    "team_id": "team-2",
    "used_client_oauth_token": true
  }
}"#;

fn populated() -> StandardLoggingPayload {
    serde_json::from_str(FULL_PAYLOAD).expect("the payload fixture parses")
}

#[test]
fn metadata_projection_carries_the_fixed_span_fields() {
    let payload = support::payload();
    let metadata =
        span_metadata(&payload, &Object::new(), &[], false).expect("the span metadata encodes");
    support::assert_fields(
        &metadata,
        r#"{
            "model_name": "gpt-4o",
            "model_provider": "unknown",
            "id": "chatcmpl-1",
            "trace_id": "trace-1",
            "cache_hit": "unknown",
            "cache_key": "unknown",
            "saved_cache_cost": 0.0,
            "guardrail_information": null,
            "is_streamed_request": false,
            "latency_metrics": {"time_to_first_token_ms": 500.0},
            "spend_metrics": {"response_cost": 0.5}
        }"#,
    );
    assert_eq!(metadata.get("_dd"), None);
}

#[test]
fn metadata_projection_carries_the_key_and_budget_fields() {
    let payload = populated();
    let metadata =
        span_metadata(&payload, &Object::new(), &[], false).expect("the span metadata encodes");
    support::assert_fields(
        &metadata,
        r#"{
            "user_api_key_hash": "hash-1",
            "user_api_key_alias": "prod-key",
            "user_api_key_spend": 1.5,
            "user_api_key_max_budget": 10.25,
            "user_api_key_budget_reset_at": "2026-01-01T00:00:00",
            "user_api_key_user_spend": 0.5,
            "user_api_key_user_max_budget": 5.25,
            "user_api_key_team_spend": 2.5,
            "user_api_key_team_max_budget": 20.25,
            "user_api_key_org_id": "org-1",
            "user_api_key_org_alias": "acme",
            "user_api_key_team_id": "team-1",
            "user_api_key_project_id": "proj-1",
            "user_api_key_project_alias": "demo",
            "user_api_key_user_id": "alice",
            "user_api_key_user_email": "alice@example.com",
            "user_api_key_team_alias": "engineering",
            "user_api_key_end_user_id": "end-1",
            "user_api_key_request_route": "/v1/chat"
        }"#,
    );
}

#[test]
fn metadata_projection_carries_the_request_payload_fields() {
    let payload = populated();
    let metadata =
        span_metadata(&payload, &Object::new(), &[], false).expect("the span metadata encodes");
    support::assert_fields(
        &metadata,
        r#"{
            "user_api_key_auth_metadata": {"source": "jwt"},
            "spend_logs_metadata": {"trace": "abc"},
            "requester_ip_address": "203.0.113.5",
            "user_agent": "curl/8",
            "requester_metadata": {"prompt": "secret"},
            "requester_custom_headers": {"x-request-id": "r-1"},
            "prompt_management_metadata": {"prompt_id": "p-1"},
            "mcp_tool_call_metadata": {"tool": "search"},
            "vector_store_request_metadata": [{"id": "vs-1"}],
            "routing_decision": {"tier": "low", "signals": ["quote"], "routed_model": "gpt-4o-mini"},
            "applied_guardrails": ["pii"],
            "usage_object": {"prompt_tokens": 2},
            "cold_storage_object_key": "s3://bucket/key",
            "team_alias": "platform",
            "team_id": "team-2",
            "used_client_oauth_token": true
        }"#,
    );
}

#[test]
fn metadata_redaction_drops_the_prompt_carrying_keys_and_the_router_signals() {
    let payload = populated();
    let router = router_span_fields(payload.metadata.routing_decision.as_ref(), true);
    let metadata = span_metadata(&payload, &router, &["user:alice".to_owned()], true)
        .expect("the span metadata encodes");
    assert_eq!(metadata.get("routing_decision"), None);
    assert_eq!(metadata.get("requester_metadata"), None);
    assert_eq!(metadata.get("prompt_management_metadata"), None);
    assert_eq!(metadata.get("mcp_tool_call_metadata"), None);
    assert_eq!(metadata.get("vector_store_request_metadata"), None);
    assert_eq!(metadata.get("router_signals"), None);
    support::assert_fields(
        &metadata,
        r#"{
            "user_api_key_user_id": "alice",
            "router_tier": "low",
            "router_escalated": false,
            "routed_model": "gpt-4o-mini",
            "_dd": {"cost_tags": ["user:alice"]}
        }"#,
    );
}

#[test]
fn metadata_carries_the_router_fields_the_caller_computed() {
    let mut payload = support::payload();
    payload.metadata.routing_decision =
        Some(support::json(r#"{"tier": "low", "signals": ["quote"]}"#));
    let router = router_span_fields(payload.metadata.routing_decision.as_ref(), false);
    let metadata = span_metadata(&payload, &router, &[], false).expect("the span metadata encodes");
    support::assert_fields(
        &metadata,
        r#"{
            "routing_decision": {"tier": "low", "signals": ["quote"]},
            "router_tier": "low",
            "router_signals": ["quote"],
            "router_escalated": false
        }"#,
    );
}

#[test]
fn metadata_redacts_a_guardrail_record_only_when_redacting() {
    let mut payload = support::payload();
    payload.guardrail_information = Some(vec![
        serde_json::from_str(r#"{"guardrail_name": "pii", "guardrail_request": {"prompt": "s"}}"#)
            .expect("the record parses"),
    ]);
    let plain =
        span_metadata(&payload, &Object::new(), &[], false).expect("the span metadata encodes");
    support::assert_fields(
        &plain,
        r#"{"guardrail_information": [{"guardrail_name": "pii", "guardrail_request": {"prompt": "s"}}]}"#,
    );
    let redacted =
        span_metadata(&payload, &Object::new(), &[], true).expect("the span metadata encodes");
    support::assert_fields(
        &redacted,
        r#"{"guardrail_information": [{"guardrail_name": "pii", "guardrail_request": "REDACTED_BY_LITELM"}]}"#,
    );
}

#[test]
fn metadata_carries_the_latency_and_spend_metrics_of_the_payload() {
    let payload = populated();
    let metadata =
        span_metadata(&payload, &Object::new(), &[], false).expect("the span metadata encodes");
    support::assert_fields(
        &metadata,
        r#"{
            "latency_metrics": {"time_to_first_token_ms": 500.0},
            "spend_metrics": {
                "response_cost": 0.5,
                "user_api_key_spend": 1.5,
                "user_api_key_max_budget": 10.25,
                "user_api_key_budget_reset_at": "2026-01-01T00:00:00+00:00"
            }
        }"#,
    );
}

#[test]
fn metadata_keeps_a_cache_hit_and_a_cache_key_the_payload_carries() {
    let mut payload = support::payload();
    payload.cache_hit = Some(false);
    payload.cache_key = Some("cache-1".to_owned());
    payload.custom_llm_provider = Some("openai".to_owned());
    let metadata =
        span_metadata(&payload, &Object::new(), &[], false).expect("the span metadata encodes");
    support::assert_fields(
        &metadata,
        r#"{"cache_hit": false, "cache_key": "cache-1", "model_provider": "openai"}"#,
    );
}

#[test]
fn metadata_marks_a_streamed_request_from_the_request_parameters() {
    let mut payload = support::payload();
    payload
        .model_parameters
        .insert("stream".to_owned(), Json::Bool(true));
    let metadata =
        span_metadata(&payload, &Object::new(), &[], false).expect("the span metadata encodes");
    assert_eq!(metadata.get("is_streamed_request"), Some(&Json::Bool(true)));
}
