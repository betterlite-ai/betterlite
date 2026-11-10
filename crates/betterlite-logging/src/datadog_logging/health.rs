use crate::datadog::payloads::standard::standard_logging_to_datadog;
use crate::datadog::payloads::status::DataDogStatus;
use crate::exporter_options::health::IntegrationHealthStatus;
use crate::fields::payload::StandardLoggingPayload;

use super::logger::DatadogLogger;
use super::send::{SendError, async_send_compressed_data};

// create_dummy_standard_logging_payload's fields, typed as StandardLoggingPayload declares them
const DUMMY_PAYLOAD: &str = r#"{
    "id": "test_id",
    "trace_id": "",
    "session_id": "",
    "call_type": "completion",
    "stream": false,
    "response_cost": 0.1,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 30,
    "prompt_tokens": 10,
    "completion_tokens": 20,
    "startTime": 1234567890.0,
    "endTime": 1234567891.0,
    "completionStartTime": 1234567890.5,
    "response_time": 0.0,
    "model_map_information": {"model_map_key": "gpt-3.5-turbo", "model_map_value": null},
    "model": "gpt-3.5-turbo",
    "model_id": "model-123",
    "model_group": "openai-gpt",
    "custom_llm_provider": "openai",
    "api_base": "https://api.openai.com",
    "metadata": {
        "user_api_key_hash": "test_hash",
        "user_api_key_alias": "test_alias",
        "user_api_key_team_id": "test_team",
        "user_api_key_user_id": "test_user",
        "user_api_key_team_alias": "test_team_alias",
        "requester_ip_address": "127.0.0.1",
        "user_api_key_end_user_id": "test_end_user"
    },
    "cache_hit": false,
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "request_model_access_groups": [],
    "requester_ip_address": "127.0.0.1",
    "messages": [{"role": "user", "content": "Hello, world!"}],
    "response": {"choices": [{"message": {"content": "Hi there!"}}]},
    "model_parameters": {"stream": true},
    "hidden_params": {}
}"#;

pub async fn async_health_check(logger: &DatadogLogger) -> IntegrationHealthStatus {
    let dummy = match serde_json::from_str::<StandardLoggingPayload>(DUMMY_PAYLOAD) {
        Ok(dummy) => dummy,
        Err(error) => return IntegrationHealthStatus::unhealthy(error.to_string()),
    };
    let payload = match standard_logging_to_datadog(
        &dummy,
        DataDogStatus::Info,
        &logger.settings.environment,
        &logger.tracer,
    ) {
        Ok(payload) => payload,
        Err(error) => return IntegrationHealthStatus::unhealthy(error.to_string()),
    };
    match async_send_compressed_data(logger, &[payload]).await {
        Ok(response) if response.status == 202 => IntegrationHealthStatus::healthy(),
        Ok(response) => IntegrationHealthStatus::unhealthy(
            str::from_utf8(&response.body)
                .unwrap_or("<non-UTF-8 body>")
                .to_owned(),
        ),
        Err(SendError::Status { detail, .. }) => IntegrationHealthStatus::unhealthy(detail),
        Err(SendError::Transport(error)) => IntegrationHealthStatus::unhealthy(error.to_string()),
        Err(SendError::Build(error)) => IntegrationHealthStatus::unhealthy(error.to_string()),
    }
}
