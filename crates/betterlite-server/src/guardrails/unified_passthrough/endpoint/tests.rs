#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::passthrough_guardrail::errors::PassthroughGuardrailError;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::BaseTranslation;
use betterlite_models::decisions::Json;

use super::PassthroughEndpointTranslation;
use crate::guardrails::unified_passthrough::tests::{
    FakeGuardrail, json_value, map_body, response_call,
};

type Adapter = PassthroughEndpointTranslation<FakeGuardrail>;

fn refusal() -> GuardrailFailure {
    GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    }
}

#[tokio::test]
async fn endpoint_input_scans_the_whole_payload_and_keeps_it() {
    let fake = FakeGuardrail::unchanged();
    let original = map_body(
        r#"{"_hidden": 1, "metadata": {"m": 1}, "litellm_logging_obj": {"x": 1}, "model": "m", "query": "hello"}"#,
    );
    let mut data = original.clone();
    let mut log_target = GuardrailLogTarget::default();
    Adapter::default()
        .process_input_messages(&mut data, &fake, Some(&mut log_target))
        .await
        .expect("the guardrail accepts");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].texts.len(), 1);
    assert!(calls[0].texts[0].contains("hello"));
    assert!(!calls[0].texts[0].contains("_hidden"));
    assert!(!calls[0].texts[0].contains("metadata"));
    assert!(!calls[0].texts[0].contains("litellm_logging_obj"));
    assert_eq!(calls[0].model.as_deref(), Some("m"));
    assert_eq!(calls[0].input_type, GuardrailInputType::Request);
    assert!(calls[0].logged);
    assert_eq!(calls[0].request_data, original);
    assert_eq!(data, original);
}

#[tokio::test]
async fn endpoint_input_failure_is_returned() {
    let fake = FakeGuardrail::failing(refusal());
    let mut data = map_body(r#"{"query": "hello"}"#);
    let error = Adapter::default()
        .process_input_messages(&mut data, &fake, None)
        .await
        .expect_err("the guardrail blocks");
    assert!(matches!(
        error,
        PassthroughGuardrailError::Guardrail(GuardrailFailure::Http { status: 400, .. })
    ));
}

#[tokio::test]
async fn endpoint_output_scans_the_response_and_returns_it_unchanged() {
    let fake = FakeGuardrail::rewriting(&["REPLACED"]);
    let response = json_value(r#"{"model": "m", "choices": [{"text": "hi"}]}"#);
    let original = response.clone();
    let mut request_data = map_body(r#"{"custom": true}"#);
    let user_key = map_body(r#"{"user_id": "u1", "_internal": "x", "empty": null}"#);
    let returned = Adapter::default()
        .process_output_response(
            response,
            &fake,
            response_call(Some(&mut request_data), Some(&user_key), None),
        )
        .await
        .expect("the guardrail accepts");
    assert_eq!(returned, original);
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(calls[0].texts.len(), 1);
    assert!(calls[0].texts[0].contains("hi"));
    assert_eq!(calls[0].model.as_deref(), Some("m"));
    let metadata = json_value(r#"{"user_api_key_user_id": "u1"}"#);
    assert_eq!(
        calls[0].request_data.get("litellm_metadata"),
        Some(&metadata)
    );
    assert_eq!(
        calls[0].request_data.get("custom"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(calls[0].request_data.get("response"), Some(&original));
    assert_eq!(request_data.get("litellm_metadata"), Some(&metadata));
    assert_eq!(request_data.get("response"), Some(&original));
}

#[tokio::test]
async fn endpoint_output_without_request_data_uses_the_response() {
    let fake = FakeGuardrail::unchanged();
    let response = json_value(r#"{"model": "m", "choices": []}"#);
    Adapter::default()
        .process_output_response(response.clone(), &fake, response_call(None, None, None))
        .await
        .expect("the guardrail accepts");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].request_data,
        map_body(r#"{"choices": [], "model": "m"}"#)
    );
    assert!(!calls[0].request_data.contains_key("litellm_metadata"));
    assert!(!calls[0].request_data.contains_key("response"));
}

#[tokio::test]
async fn endpoint_output_non_object_skips_the_guardrail() {
    let fake = FakeGuardrail::unchanged();
    let response = GuardrailValue::Array(vec![GuardrailValue::Text("hi".to_owned())]);
    let returned = Adapter::default()
        .process_output_response(response.clone(), &fake, response_call(None, None, None))
        .await
        .expect("nothing to scan");
    assert_eq!(returned, response);
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn endpoint_output_failure_is_returned() {
    let fake = FakeGuardrail::failing(refusal());
    let response = json_value(r#"{"output": {"message": {"content": [{"text": "hi"}]}}}"#);
    let error = Adapter::default()
        .process_output_response(response, &fake, response_call(None, None, None))
        .await
        .expect_err("the guardrail blocks");
    assert!(matches!(
        error,
        PassthroughGuardrailError::Guardrail(GuardrailFailure::Http { status: 400, .. })
    ));
}

#[tokio::test]
async fn endpoint_default_methods_keep_the_base_behavior() {
    let adapter = Adapter::default();
    let fake = FakeGuardrail::unchanged();
    const {
        assert!(!<Adapter as BaseTranslation>::DELIVERS_ENDED_STREAM_REWRITES);
        assert!(!<Adapter as BaseTranslation>::ASSEMBLES_STREAMED_RESPONSE);
    }
    let response = json_value(r#"{"ok": true}"#);
    let mut responses = vec![response.clone()];
    assert_eq!(adapter.post_call_hook_response(response.clone()), response);
    assert_eq!(adapter.get_streaming_scan_key(&responses), None);
    assert!(adapter.get_structured_messages(&map_body("{}")).is_none());
    assert!(
        adapter
            .extract_request_tool_names(&map_body("{}"))
            .is_empty()
    );
    adapter
        .process_streaming_response(&mut responses, &fake, response_call(None, None, None))
        .await
        .expect("the default keeps the list");
    assert_eq!(responses, vec![response.clone()]);
    let failure = PassthroughGuardrailError::Guardrail(refusal());
    assert!(
        adapter
            .build_block_sse_chunks(&failure, true, None)
            .is_none()
    );
    assert!(adapter.build_stream_error_items(&failure, None).is_none());
}

#[tokio::test]
async fn endpoint_output_sends_only_the_configured_response_fields() {
    let fake = FakeGuardrail::unchanged();
    let config: Json = serde_json::from_str(r#"{"g1": {"response_fields": ["output"]}}"#)
        .expect("valid JSON fixture");
    let response = json_value(r#"{"model": "m", "output": "hello", "secret": "hide"}"#);
    let mut log_target = GuardrailLogTarget::default();
    Adapter::new(Some("g1".to_owned()), Some(config))
        .process_output_response(
            response,
            &fake,
            response_call(None, None, Some(&mut log_target)),
        )
        .await
        .expect("the guardrail accepts");
    assert_eq!(fake.calls()[0].texts, vec!["hello".to_owned()]);
}

#[tokio::test]
async fn endpoint_input_sends_only_the_configured_request_fields() {
    let fake = FakeGuardrail::unchanged();
    let config: Json = serde_json::from_str(r#"{"g1": {"request_fields": ["query"]}}"#)
        .expect("valid JSON fixture");
    let mut data = map_body(r#"{"model": "m", "query": "hello", "secret": "hide"}"#);
    let mut log_target = GuardrailLogTarget::default();
    Adapter::new(Some("g1".to_owned()), Some(config))
        .process_input_messages(&mut data, &fake, Some(&mut log_target))
        .await
        .expect("the guardrail accepts");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].texts, vec!["hello".to_owned()]);
    assert_eq!(calls[0].model.as_deref(), Some("m"));
    assert_eq!(
        data.get("secret"),
        Some(&GuardrailValue::Text("hide".to_owned()))
    );
}
