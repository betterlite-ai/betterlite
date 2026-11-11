use crate::fields::payload::StandardLoggingPayload;

// create_dummy_standard_logging_payload's fields, typed as StandardLoggingPayload declares them
pub const HEALTH_CHECK_PAYLOAD: &str = r#"{
    "id": "sqs-health-check",
    "trace_id": "",
    "session_id": "",
    "call_type": "completion",
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
    "model_map_information": {},
    "model": "gpt-3.5-turbo",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

pub trait HealthPayloadFactory: Send + Sync {
    fn build(&self) -> Result<StandardLoggingPayload, serde_json::Error>;
}

pub struct DummyHealthPayload;

impl HealthPayloadFactory for DummyHealthPayload {
    fn build(&self) -> Result<StandardLoggingPayload, serde_json::Error> {
        serde_json::from_str(HEALTH_CHECK_PAYLOAD)
    }
}

mod tests;
