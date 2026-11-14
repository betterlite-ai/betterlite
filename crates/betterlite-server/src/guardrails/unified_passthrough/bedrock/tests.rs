#![cfg(test)]

use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::passthrough_guardrail::errors::PassthroughGuardrailError;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_translation::BaseTranslation;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{BedrockPassthroughError, BedrockPassthroughTranslation};
use crate::guardrails::bedrock_passthrough::event_stream::frames::parse_event_stream;
use crate::guardrails::bedrock_passthrough::output::OutputTranslationError;
use crate::guardrails::bedrock_passthrough::request::RequestTranslationError;
use crate::guardrails::bedrock_passthrough::text_paths::SkipFlags;
use crate::guardrails::unified_passthrough::tests::{
    FakeGuardrail, TestHook, json_value, map_body, response_call,
};

type BedrockAdapter = BedrockPassthroughTranslation<FakeGuardrail, TestHook>;

const CONVERSE: &str = "https://bedrock-runtime.us-east-1.amazonaws.com/model/m/converse";

fn adapter(skip: SkipFlags) -> BedrockAdapter {
    BedrockPassthroughTranslation::new(TestHook::echoing(), skip, None, None)
}

fn refusal() -> GuardrailFailure {
    GuardrailFailure::Http {
        status: 400,
        detail: "blocked".to_owned(),
    }
}

fn converse_input() -> String {
    format!(
        r#"{{"endpoint": "{CONVERSE}", "model": "m", "data": {{"messages": [{{"role": "user", "content": [{{"text": "hello"}}, {{"toolUse": {{"toolUseId": "t1", "name": "n", "input": {{"q": "tool text"}}}}}}]}}], "system": [{{"text": "be nice"}}]}}}}"#
    )
}

fn delta_frame(delta: &str) -> Vec<u8> {
    let payload = format!("{{\"delta\": {delta}}}");
    let mut message = Message::new(payload.into_bytes());
    for (name, value) in [
        (":event-type", "contentBlockDelta"),
        (":message-type", "event"),
    ] {
        message = message.add_header(Header::new(
            (*name).to_owned(),
            HeaderValue::String(StrBytes::from((*value).to_owned())),
        ));
    }
    let mut buffer = Vec::new();
    write_message_to(&message, &mut buffer).expect("writes a frame");
    buffer
}

#[rstest]
#[case::neither(SkipFlags::default(), &["be nice", "hello", "tool text"])]
#[case::system_only(SkipFlags { system: true, tool: false }, &["hello", "tool text"])]
#[case::tool_only(SkipFlags { system: false, tool: true }, &["be nice", "hello"])]
#[tokio::test]
async fn bedrock_converse_input_uses_the_stored_skip_flags(
    #[case] skip: SkipFlags,
    #[case] expected: &[&str],
) {
    let fake = FakeGuardrail::unchanged();
    let mut data = map_body(&converse_input());
    adapter(skip)
        .process_input_messages(&mut data, &fake, None)
        .await
        .expect("the guardrail accepts");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    let texts: Vec<&str> = calls[0].texts.iter().map(String::as_str).collect();
    assert_eq!(texts, expected);
    assert_eq!(calls[0].model.as_deref(), Some("m"));
    assert_eq!(calls[0].input_type, GuardrailInputType::Request);
}

#[tokio::test]
async fn bedrock_generic_input_uses_the_configured_request_fields() {
    let fake = FakeGuardrail::unchanged();
    let config: Json = serde_json::from_str(r#"{"g1": {"request_fields": ["prompt"]}}"#)
        .expect("valid JSON fixture");
    let adapter = BedrockAdapter::new(
        TestHook::echoing(),
        SkipFlags::default(),
        Some("g1".to_owned()),
        Some(config),
    );
    let mut data = map_body(
        r#"{"endpoint": "https://x/model/m/invoke", "model": "m", "prompt": "hello", "secret": "hide"}"#,
    );
    adapter
        .process_input_messages(&mut data, &fake, None)
        .await
        .expect("the guardrail accepts");
    assert_eq!(fake.calls()[0].texts, vec!["hello".to_owned()]);
}

#[tokio::test]
async fn bedrock_converse_input_writes_the_guardrailed_texts_back() {
    let fake = FakeGuardrail::rewriting(&["HELLO"]);
    let mut data = map_body(&format!(
        r#"{{"endpoint": "{CONVERSE}", "model": "m", "data": {{"messages": [{{"role": "user", "content": [{{"text": "hello"}}]}}]}}}}"#
    ));
    adapter(SkipFlags::default())
        .process_input_messages(&mut data, &fake, None)
        .await
        .expect("the guardrail accepts");
    assert_eq!(fake.calls()[0].texts, vec!["hello".to_owned()]);
    assert_eq!(
        data,
        map_body(&format!(
            r#"{{"endpoint": "{CONVERSE}", "model": "m", "data": {{"messages": [{{"role": "user", "content": [{{"text": "HELLO"}}]}}]}}}}"#
        ))
    );
}

#[tokio::test]
async fn bedrock_generic_input_runs_the_endpoint_wrapper() {
    let fake = FakeGuardrail::unchanged();
    let original =
        map_body(r#"{"endpoint": "https://x/model/m/invoke", "model": "m", "prompt": "hello"}"#);
    let mut data = original.clone();
    adapter(SkipFlags::default())
        .process_input_messages(&mut data, &fake, None)
        .await
        .expect("the guardrail accepts");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].texts[0].contains("hello"));
    assert_eq!(calls[0].model.as_deref(), Some("m"));
    assert_eq!(calls[0].input_type, GuardrailInputType::Request);
    assert_eq!(calls[0].request_data, original);
    assert_eq!(data, original);
}

#[tokio::test]
async fn bedrock_converse_output_uses_the_caller_key_metadata() {
    let fake = FakeGuardrail::rewriting(&["HELLO"]);
    let user_key = map_body(r#"{"user_id": "u1", "_internal": "x", "empty": null}"#);
    let metadata = map_body(r#"{"user_api_key_user_id": "u1"}"#);
    let response = json_value(
        r#"{"output": {"message": {"role": "assistant", "content": [{"text": "hello"}]}}}"#,
    );
    let mut request_data = map_body(&format!(r#"{{"endpoint": "{CONVERSE}", "model": "m"}}"#));
    let caller_data = request_data.clone();
    let returned = adapter(SkipFlags::default())
        .process_output_response(
            response,
            &fake,
            response_call(Some(&mut request_data), Some(&user_key), None),
        )
        .await
        .expect("the guardrail accepts");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].texts, vec!["hello".to_owned()]);
    assert_eq!(calls[0].model.as_deref(), Some("m"));
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    assert_eq!(
        calls[0].request_data.get("litellm_metadata"),
        Some(&GuardrailValue::Object(metadata))
    );
    assert!(calls[0].request_data.get("response").is_none());
    assert_eq!(request_data, caller_data);
    assert_eq!(
        returned,
        json_value(
            r#"{"output": {"message": {"role": "assistant", "content": [{"text": "HELLO"}]}}}"#
        )
    );
}

#[tokio::test]
async fn bedrock_converse_output_without_a_key_adds_no_metadata() {
    let fake = FakeGuardrail::unchanged();
    let response = json_value(r#"{"output": {"message": {"content": [{"text": "hi"}]}}}"#);
    let mut request_data = map_body(&format!(r#"{{"endpoint": "{CONVERSE}"}}"#));
    let caller_data = request_data.clone();
    adapter(SkipFlags::default())
        .process_output_response(
            response,
            &fake,
            response_call(Some(&mut request_data), None, None),
        )
        .await
        .expect("the guardrail accepts");
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].request_data.get("litellm_metadata").is_none());
    assert_eq!(request_data, caller_data);
}

#[tokio::test]
async fn bedrock_generic_output_runs_the_endpoint_wrapper() {
    let fake = FakeGuardrail::rewriting(&["REPLACED"]);
    let response = json_value(r#"{"model": "m", "completion": "hello"}"#);
    let mut request_data = map_body(r#"{"endpoint": "https://x/model/m/invoke"}"#);
    let user_key = map_body(r#"{"user_id": "u1"}"#);
    let returned = adapter(SkipFlags::default())
        .process_output_response(
            response.clone(),
            &fake,
            response_call(Some(&mut request_data), Some(&user_key), None),
        )
        .await
        .expect("the guardrail accepts");
    assert_eq!(returned, response);
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].texts[0].contains("hello"));
    assert_eq!(calls[0].input_type, GuardrailInputType::Response);
    let metadata = json_value(r#"{"user_api_key_user_id": "u1"}"#);
    assert_eq!(
        calls[0].request_data.get("litellm_metadata"),
        Some(&metadata)
    );
    assert_eq!(request_data.get("litellm_metadata"), Some(&metadata));
    assert_eq!(request_data.get("response"), Some(&response));
}

#[tokio::test]
async fn bedrock_converse_input_failure_is_a_request_error() {
    let fake = FakeGuardrail::failing(refusal());
    let mut data = map_body(&format!(
        r#"{{"endpoint": "{CONVERSE}", "data": {{"messages": [{{"content": [{{"text": "hello"}}]}}]}}}}"#
    ));
    let error = adapter(SkipFlags::default())
        .process_input_messages(&mut data, &fake, None)
        .await
        .expect_err("the guardrail blocks");
    assert!(matches!(
        error,
        BedrockPassthroughError::Request(RequestTranslationError::Guardrail(
            GuardrailFailure::Http { status: 400, .. }
        ))
    ));
}

#[tokio::test]
async fn bedrock_converse_output_failure_is_an_output_error() {
    let fake = FakeGuardrail::failing(refusal());
    let response = json_value(r#"{"output": {"message": {"content": [{"text": "hi"}]}}}"#);
    let mut request_data = map_body(&format!(r#"{{"endpoint": "{CONVERSE}"}}"#));
    let error = adapter(SkipFlags::default())
        .process_output_response(
            response,
            &fake,
            response_call(Some(&mut request_data), None, None),
        )
        .await
        .expect_err("the guardrail blocks");
    assert!(matches!(
        error,
        BedrockPassthroughError::Output(OutputTranslationError::Guardrail(
            GuardrailFailure::Http { status: 400, .. }
        ))
    ));
}

#[tokio::test]
async fn bedrock_generic_input_failure_is_an_endpoint_error() {
    let fake = FakeGuardrail::failing(refusal());
    let mut data = map_body(r#"{"endpoint": "https://x/model/m/invoke", "prompt": "hello"}"#);
    let error = adapter(SkipFlags::default())
        .process_input_messages(&mut data, &fake, None)
        .await
        .expect_err("the guardrail blocks");
    assert!(matches!(
        error,
        BedrockPassthroughError::Endpoint(PassthroughGuardrailError::Guardrail(
            GuardrailFailure::Http { status: 400, .. }
        ))
    ));
}

#[tokio::test]
async fn bedrock_default_methods_keep_the_base_behavior() {
    let subject = adapter(SkipFlags::default());
    let fake = FakeGuardrail::unchanged();
    assert!(!<BedrockAdapter as BaseTranslation>::DELIVERS_ENDED_STREAM_REWRITES);
    assert!(!<BedrockAdapter as BaseTranslation>::ASSEMBLES_STREAMED_RESPONSE);
    let response = json_value(r#"{"ok": true}"#);
    assert_eq!(subject.post_call_hook_response(response.clone()), response);
    assert!(
        subject
            .get_streaming_scan_key(&[response.clone()])
            .is_none()
    );
    let mut responses = vec![response.clone()];
    subject
        .process_streaming_response(&mut responses, &fake, response_call(None, None, None))
        .await
        .expect("the default keeps the list");
    assert_eq!(responses, vec![response]);
}

#[tokio::test]
async fn bedrock_event_stream_runs_the_stored_hook() {
    let body = delta_frame(r#"{"text": "hello"}"#);
    let data = map_body(r#"{"model": "m"}"#);
    let key = map_body(r#"{"user_id": "u1"}"#);
    let adapter = BedrockAdapter::new(
        TestHook::replacing("HELLO"),
        SkipFlags::default(),
        None,
        None,
    );
    let output = adapter
        .de_anonymize_event_stream(&body, &key, &data)
        .await
        .expect("the hook answers");
    let parsed = parse_event_stream(&output);
    assert_eq!(parsed.frames.len(), 1);
    assert_eq!(parsed.frames[0].texts.len(), 1);
    assert_eq!(parsed.frames[0].texts[0].1, "HELLO");
    let seen = adapter.hook.seen();
    assert_eq!(seen.data, Some(data));
    assert_eq!(seen.key, Some(key));
    assert_eq!(
        seen.response,
        Some(json_value(
            r#"{"output": {"message": {"role": "assistant", "content": [{"text": "hello"}]}}, "stopReason": "end_turn"}"#
        ))
    );
}
