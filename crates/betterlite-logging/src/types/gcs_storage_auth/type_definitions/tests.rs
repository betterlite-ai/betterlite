#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;
use secrecy::SecretString;

use super::{GcsLogQueueItem, GcsLoggingConfig, GcsLoggingKwargs, GcsLoggingPayload};
use crate::fields::payload::StandardLoggingPayload;
use crate::gcs_mock::auth::GcsMockAuth;

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": null,
    "call_type": "completion",
    "stream": false,
    "response_cost": 0.25,
    "cost_breakdown": null,
    "autorouter_savings_estimate": null,
    "autorouter_baseline_observation": null,
    "response_cost_failure_debug_info": null,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "custom_llm_provider": "openai",
    "total_tokens": 3,
    "prompt_tokens": 1,
    "completion_tokens": 2,
    "startTime": 1704067200.0,
    "endTime": 1704067201.0,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {},
    "model": "gpt-4o",
    "model_id": null,
    "model_group": null,
    "api_base": "https://api.openai.com",
    "metadata": {},
    "cache_hit": null,
    "cache_key": null,
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "end_user": null,
    "requester_ip_address": null,
    "user_agent": null,
    "messages": null,
    "response": null,
    "error_str": null,
    "error_information": null,
    "model_parameters": {},
    "hidden_params": {},
    "guardrail_information": null,
    "standard_built_in_tools_params": null
}"#;

fn payload() -> StandardLoggingPayload {
    serde_json::from_str(PAYLOAD).expect("the fixture is a standard logging payload")
}

#[test]
fn the_logging_config_carries_the_bucket_the_instance_and_the_service_account() {
    let config = GcsLoggingConfig {
        bucket_name: "bucket".to_owned(),
        vertex_instance: Arc::new(GcsMockAuth {
            real: (),
            installed: false,
        }),
        path_service_account: Some(SecretString::from("account")),
    };
    assert_eq!(config.bucket_name, "bucket");
    assert_eq!(
        config
            .path_service_account
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("account")
    );
    assert!(!config.vertex_instance.installed);
}

#[test]
fn the_queue_item_carries_the_payload_the_kwargs_and_the_response() {
    let item = GcsLogQueueItem {
        payload: payload(),
        kwargs: GcsLoggingKwargs::default(),
        response_obj: Some(Json::Text("response".to_owned())),
    };
    assert_eq!(item.payload.id, "chatcmpl-1");
    assert!(item.kwargs.dynamic_params.is_none());
    assert_eq!(item.response_obj, Some(Json::Text("response".to_owned())));
}

#[test]
fn a_text_payload_stays_text_and_a_standard_payload_keeps_its_fields() {
    let text = GcsLoggingPayload::Text("{}".to_owned());
    let standard = GcsLoggingPayload::Standard(Box::new(payload()));
    let GcsLoggingPayload::Text(text_value) = text else {
        panic!("the text payload stayed text");
    };
    assert_eq!(text_value, "{}");
    let GcsLoggingPayload::Standard(standard_value) = standard else {
        panic!("the standard payload stayed standard");
    };
    assert_eq!(standard_value.model, "gpt-4o");
}
