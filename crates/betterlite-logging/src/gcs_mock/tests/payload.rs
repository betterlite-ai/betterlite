use crate::gcs_mock::payload::{MOCK_LOGGING_PAYLOAD, MOCK_STORAGE_OBJECT};

// LiteLLM json.dumps's this dict; integer zeroes stand in for its 0.0 floats, which the compatibility policy allows
const EXPECTED_PAYLOAD: &str = concat!(
    r#"{"id":"mock-request-id","trace_id":"mock-trace-id","call_type":"completion","stream":false,"#,
    r#""response_cost":0,"status":"success","status_fields":{"llm_api_status":"success"},"#,
    r#""custom_llm_provider":"mock","total_tokens":0,"prompt_tokens":0,"completion_tokens":0,"#,
    r#""startTime":0,"endTime":0,"completionStartTime":0,"response_time":0,"#,
    r#""model_map_information":{"model":"mock-model"},"model":"mock-model","model_id":null,"#,
    r#""model_group":null,"api_base":"https://api.mock.com","metadata":{},"cache_hit":null,"#,
    r#""cache_key":null,"saved_cache_cost":0,"request_tags":[],"end_user":null,"#,
    r#""requester_ip_address":null,"messages":null,"response":null,"error_str":null,"#,
    r#""error_information":null,"model_parameters":{},"hidden_params":{},"#,
    r#""guardrail_information":null,"standard_built_in_tools_params":null}"#
);

#[test]
fn mock_payload_serializes_every_litellm_field() {
    assert_eq!(
        serde_json::to_string(&MOCK_LOGGING_PAYLOAD).unwrap(),
        EXPECTED_PAYLOAD
    );
}

#[test]
fn mock_storage_object_is_the_factory_response() {
    assert_eq!(
        serde_json::to_string(&MOCK_STORAGE_OBJECT).unwrap(),
        r#"{"kind":"storage#object","name":"mock-object"}"#
    );
}
