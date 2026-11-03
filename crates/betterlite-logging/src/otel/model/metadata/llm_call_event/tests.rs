#![cfg(test)]
use crate::otel::model::metadata::llm_call_event::LLMCallEvent;
use crate::otel::model::metadata::wire::{callback::Callback, decode};
use crate::otel::model::utils::coercion::OtelModelValue;
use rust_decimal::Decimal;

#[test]
fn parses_live_callback_fields() {
    let kwargs: Callback = serde_json::from_str(r#"{"litellm_call_id":"call","model":"model","call_type":"completion","api_call_start_time":10.0,"completion_start_time":10.5,"optional_params":{"stream":true},"standard_callback_dynamic_params":{"endpoint":"x"}}"#).expect("test JSON is valid");
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    assert_eq!(event.call_id.as_deref(), Some("call"));
    assert_eq!(event.provisional_span_name, "chat model");
    assert_eq!(event.time_to_first_chunk_seconds, Some(Decimal::new(5, 1)));
    assert!(event.upstream_started);
    assert!(event.dynamic_params.is_some());
}

#[test]
fn closed_payload_wins_and_rejected_calls_have_no_upstream() {
    let kwargs: Callback = serde_json::from_str(r#"{"litellm_call_id":"live","standard_logging_object":{"id":"closed","metadata":{"user_api_key_auth_metadata":{"tenant":"trusted"}}},"litellm_no_upstream_llm_call":true,"litellm_params":{"metadata":{"session_id":"conversation","trace_name":"caller"}}}"#).expect("test JSON is valid");
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    assert_eq!(event.call_id.as_deref(), Some("closed"));
    assert!(event.payload.is_some());
    assert!(event.is_no_upstream_call);
    assert!(!event.upstream_started);
    assert_eq!(event.time_to_first_chunk_seconds, None);
    assert_eq!(event.session_id.as_deref(), Some("conversation"));
    assert_eq!(event.trace.name.as_deref(), Some("caller"));
    assert_eq!(
        event
            .auth_metadata
            .unwrap()
            .get("tenant")
            .map(String::as_str),
        Some("trusted")
    );
}

#[rstest::rstest]
#[case::stream(r#"{"optional_params":{"stream":"yes"}}"#, "optional_params.stream")]
#[case::payload(
    r#"{"standard_logging_object":{"model":false}}"#,
    "standard_logging_object.model"
)]
#[case::timestamp(r#"{"api_call_start_time":"invalid"}"#, "api_call_start_time")]
#[case::session(
    r#"{"litellm_params":{"metadata":{"session_id":42}}}"#,
    "litellm_params.metadata.session_id"
)]
#[case::tokens(
    r#"{"standard_logging_object":{"prompt_tokens":"many"}}"#,
    "standard_logging_object.prompt_tokens"
)]
#[case::cost(
    r#"{"standard_logging_object":{"response_cost":false}}"#,
    "standard_logging_object.response_cost"
)]
#[case::response_model(
    r#"{"standard_logging_object":{"response":{"model":false}}}"#,
    "standard_logging_object.response"
)]
fn rejects_malformed_stream(#[case] source: &str, #[case] field: &str) {
    assert!(
        decode::<Callback>(source)
            .unwrap_err()
            .to_string()
            .contains(field)
    );
}

#[test]
fn null_callback_fields_take_defaults() {
    let kwargs: Callback = decode(r#"{"litellm_params":null,"optional_params":{"stream":null},"litellm_no_upstream_llm_call":null}"#).unwrap();
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    assert!(!event.is_no_upstream_call);
    assert!(!event.upstream_started);
    assert_eq!(event.time_to_first_chunk_seconds, None);
}

#[test]
fn serialized_latency_is_an_exact_json_number() {
    let kwargs: Callback = decode(r#"{"optional_params":{"stream":true},"api_call_start_time":0,"completion_start_time":0.123456789012345678}"#).unwrap();
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    let serialized = serde_json::to_string(&event).unwrap();
    let decoded: Latency = serde_json::from_str(&serialized).unwrap();
    assert_eq!(
        decoded.time_to_first_chunk_seconds.to_string(),
        "0.123456789012345678"
    );
}

#[derive(serde::Deserialize)]
struct Latency {
    time_to_first_chunk_seconds: serde_json::Number,
}

#[test]
fn callback_preserves_usage_cost_and_content() {
    let source = r#"{
        "prompt_tokens":17,"completion_tokens":5,"total_tokens":22,
        "response_cost":0.000000000000000123,
        "response":{"model":"deployment","choices":[{"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}]},
        "messages":[{"role":"user","content":"question"}],
        "cost_breakdown":{"input_cost":0.000000000000000100,"output_cost":0.000000000000000023},
        "custom_llm_provider":"openai",
        "hidden_params":{"cache_key":"cached","usage_object":{"prompt_tokens_details":{"cached_tokens":4}}}
    }"#;
    let kwargs: Callback = decode(&format!(r#"{{"standard_logging_object":{source}}}"#)).unwrap();
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    let serialized = serde_json::to_string(&event.payload.unwrap()).unwrap();
    let captured: PayloadCapture = serde_json::from_str(&serialized).unwrap();
    let expected: PayloadCapture = serde_json::from_str(source).unwrap();
    assert_eq!(
        serde_json::to_string(&captured).unwrap(),
        serde_json::to_string(&expected).unwrap()
    );
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PayloadCapture {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    response_cost: Decimal,
    response: OtelModelValue,
    messages: OtelModelValue,
    cost_breakdown: OtelModelValue,
    custom_llm_provider: String,
    hidden_params: HiddenCapture,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct HiddenCapture {
    cache_key: String,
    usage_object: OtelModelValue,
}

#[rstest::rstest]
#[case::object(r#"{"choices":[{"text":"answer"}],"usage":{"total_tokens":7}}"#)]
#[case::text(r#""answer""#)]
#[case::list(r#"[{"content":"answer"},null,[1,"nested"]]"#)]
#[case::null("null")]
fn callback_preserves_response_variants(#[case] source: &str) {
    let kwargs: Callback = decode(&format!(
        r#"{{"standard_logging_object":{{"response":{source}}}}}"#
    ))
    .unwrap();
    let event = LLMCallEvent::from_dict(&kwargs).unwrap();
    let serialized = serde_json::to_string(&event.payload.unwrap()).unwrap();
    let captured: ResponseCapture = serde_json::from_str(&serialized).unwrap();
    let expected: OtelModelValue = serde_json::from_str(source).unwrap();
    assert_eq!(
        serde_json::to_string(&captured.response).unwrap(),
        serde_json::to_string(&expected).unwrap()
    );
}

#[derive(serde::Deserialize)]
struct ResponseCapture {
    response: OtelModelValue,
}
