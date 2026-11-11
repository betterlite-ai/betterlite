use betterlite_logging::azure_sentinel_logging::audit::StandardAuditLogPayload;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use serde_json::json;

pub(super) fn model_payload(
    id: &str,
    failure: bool,
    pad: usize,
) -> Result<StandardLoggingPayload, serde_json::Error> {
    let status = if failure { "failure" } else { "success" };
    let parameters = if pad == 0 {
        json!({})
    } else {
        json!({ "pad": "x".repeat(pad) })
    };
    serde_json::from_value(json!({
        "id": id,
        "trace_id": "trace-1",
        "session_id": "session-1",
        "call_type": "completion",
        "response_cost": 0.5,
        "status": status,
        "status_fields": {"llm_api_status": status, "guardrail_status": "not_run"},
        "total_tokens": 5,
        "prompt_tokens": 2,
        "completion_tokens": 3,
        "startTime": 1_704_067_200.0,
        "endTime": 1_704_067_201.5,
        "completionStartTime": 1_704_067_200.5,
        "response_time": 1.0,
        "model_map_information": {},
        "model": "gpt-4o",
        "api_base": "https://api.openai.com",
        "metadata": {},
        "saved_cache_cost": 0.0,
        "request_tags": [],
        "model_parameters": parameters,
        "hidden_params": {},
    }))
}

pub(super) fn audit_payload(id: &str) -> StandardAuditLogPayload {
    StandardAuditLogPayload {
        id: id.to_owned(),
        updated_at: "2024-01-01T00:00:00Z".to_owned(),
        changed_by: "admin".to_owned(),
        hashed_changed_by_api_key: "hashed-key".to_owned(),
        action: "created".to_owned(),
        table_name: "LiteLLM_VerificationToken".to_owned(),
        object_id: "object-1".to_owned(),
        before_value: None,
        updated_values: Some("{}".to_owned()),
    }
}
