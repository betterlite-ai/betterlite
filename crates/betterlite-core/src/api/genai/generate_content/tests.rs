#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::chat::{MessageContent, Role};
use betterlite_prompts::arguments::json::Json;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use serde_json::Number;

use self::support::{
    ADAPTER_MODEL, DEFAULT_MOCK_TEXT, Entry, Harness, MOCK_TEMPLATE, MOCK_TEXT, NATIVE_MODEL,
    NATIVE_REPLY, SSE, Scripted, Source, Wire, contents, json, object, request, stream_output,
};
use crate::api::genai::chat_call::{
    ChatCallError, ForwardedFields, GenerateContentOutcome as AdapterOutcome, Operation,
};
use crate::api::genai::generate_content::error::GenerateContentError;
use crate::api::genai::generate_content::{GenerateContentAnswer, mock_generate_content_response};
use crate::api::genai_http::error::GenAiHttpError;
use crate::api::genai_http::service::{AsyncGenerateContentOutcome, GenerateContentOutcome};
use crate::genai_setup::error::GenAiSetupError;

const ERROR_BODY: &str = r#"{"error":{"code":429,"message":"slow down"}}"#;
const TOOLS: &str = r#"[{"functionDeclarations":[{"name":"lookup"}]}]"#;

#[tokio::test]
async fn dispatch_generate_content_moves_generation_config_into_the_config() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object(r#"{"generationConfig":{"temperature":0.4}}"#);
    let request = request(ADAPTER_MODEL, &contents, None, &kwargs);

    let dispatch = harness.call(Entry::Generate, &request).await;

    let GenerateContentAnswer::Adapter(AdapterOutcome::Response(response)) =
        dispatch.answer.expect("the call succeeds")
    else {
        panic!("an adapter call answers the translated response");
    };
    let seen = harness.executor.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, Operation::Completion);
    let expected: Number = serde_json::from_str("0.4").expect("the expected temperature parses");
    assert_eq!(seen[0].1.request.temperature, Some(expected));
    assert_eq!(seen[0].1.request.model.as_deref(), Some(ADAPTER_MODEL));
    assert_eq!(
        response.candidates[0].finish_reason.as_deref(),
        Some("STOP")
    );
    assert_eq!(
        dispatch.state.litellm_params.get("agenerate_content"),
        Some(&DecisionJson::Bool(false))
    );
}

#[tokio::test]
async fn dispatch_generate_content_keeps_the_given_config_over_generation_config() {
    let harness = Harness::new();
    let contents = contents();
    let config = object(r#"{"temperature":0.2}"#);
    let kwargs = object(r#"{"generationConfig":{"temperature":0.9}}"#);
    let mut request = request(ADAPTER_MODEL, &contents, None, &kwargs);
    request.config = Some(&config);

    harness
        .call(Entry::Generate, &request)
        .await
        .answer
        .expect("the call succeeds");

    let seen = harness.executor.seen();
    let expected: Number = serde_json::from_str("0.2").expect("the expected temperature parses");
    assert_eq!(seen[0].1.request.temperature, Some(expected));
}

#[tokio::test]
async fn dispatch_generate_content_marks_the_async_entry_key_and_awaits_async() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object(r#"{"agenerate_content":true}"#);
    let request = request(ADAPTER_MODEL, &contents, None, &kwargs);

    let dispatch = harness.call(Entry::Generate, &request).await;

    assert!(matches!(
        dispatch.answer.expect("the call succeeds"),
        GenerateContentAnswer::Adapter(AdapterOutcome::Response(_))
    ));
    assert_eq!(
        dispatch.state.litellm_params.get("agenerate_content"),
        Some(&DecisionJson::Bool(true))
    );
    assert_eq!(harness.executor.seen()[0].0, Operation::Acompletion);
}

#[tokio::test]
async fn dispatch_agenerate_content_resolves_the_provider_before_the_inner_entry() {
    let harness = Harness::new();
    let contents = contents();
    // max_retries would fail the inner entry's own kwargs parse, so the resolution error proves the order
    let kwargs = object(r#"{"max_retries":"many"}"#);
    let request = request("missing-model", &contents, None, &kwargs);

    let dispatch = harness.call(Entry::Agenerate, &request).await;

    let Err(failure) = dispatch.answer else {
        panic!("the provider resolution fails first");
    };
    assert!(matches!(
        failure.error,
        GenerateContentError::Resolve(ResolutionError::MissingProvider { .. })
    ));
    assert_eq!(failure.class.metadata().status_code, 400_u16);
    assert_eq!(harness.executor.seen().len(), 0);
}

#[tokio::test]
async fn dispatch_adapter_call_forwards_metadata_headers_and_proxy_request() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs =
        object(r#"{"metadata":{"user":"u1"},"proxy_server_request":{"url":"https://proxy.test"}}"#);
    let headers = IndexMap::from([(String::from("x-extra"), String::from("1"))]);
    let mut request = request(ADAPTER_MODEL, &contents, None, &kwargs);
    request.extra_headers = Some(&headers);

    harness
        .call(Entry::Generate, &request)
        .await
        .answer
        .expect("the call succeeds");

    let metadata: IndexMap<String, DecisionJson> =
        serde_json::from_str(r#"{"user":"u1"}"#).expect("the metadata fixture parses");
    let proxy: IndexMap<String, DecisionJson> =
        serde_json::from_str(r#"{"url":"https://proxy.test"}"#).expect("the proxy fixture parses");
    let seen = harness.executor.seen();
    assert_eq!(
        seen[0].1.forwarded,
        ForwardedFields {
            metadata: Some(metadata),
            extra_headers: Some(headers.clone()),
            proxy_server_request: Some(proxy),
        }
    );
}

#[tokio::test]
async fn dispatch_adapter_call_forwards_params_and_narrows_a_negative_retry_count() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object(r#"{"max_retries":3,"organization":"org-1"}"#);
    let request = request(ADAPTER_MODEL, &contents, None, &kwargs);

    harness
        .call(Entry::Generate, &request)
        .await
        .answer
        .expect("the call succeeds");

    let seen = harness.executor.seen();
    let params = seen[0]
        .1
        .litellm_params
        .as_ref()
        .expect("the adapter call carries the params");
    assert_eq!(params.max_retries, Some(3_u32));
    assert_eq!(params.organization.as_deref(), Some("org-1"));

    let negative = Harness::new();
    let kwargs = object(r#"{"max_retries":-1}"#);
    let request = request(ADAPTER_MODEL, &contents, None, &kwargs);
    negative
        .call(Entry::Generate, &request)
        .await
        .answer
        .expect("the call succeeds");
    let seen = negative.executor.seen();
    let params = seen[0]
        .1
        .litellm_params
        .as_ref()
        .expect("the adapter call carries the params");
    assert_eq!(params.max_retries, Some(0_u32));
}

#[tokio::test]
async fn dispatch_adapter_call_falls_through_an_empty_system_instruction() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs =
        object(r#"{"systemInstruction":"","system_instruction":{"parts":[{"text":"be nice"}]}}"#);
    let request = request(ADAPTER_MODEL, &contents, None, &kwargs);

    harness
        .call(Entry::Generate, &request)
        .await
        .answer
        .expect("the call succeeds");

    let seen = harness.executor.seen();
    assert_eq!(seen[0].1.request.messages[0].role, Role::System);
    assert_eq!(
        seen[0].1.request.messages[0].content,
        Some(MessageContent::Text(String::from("be nice")))
    );
}

#[tokio::test]
async fn dispatch_sync_stream_passes_stream_and_no_tools() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object("{}");
    let tools = json(TOOLS);
    let mut request = request(ADAPTER_MODEL, &contents, None, &kwargs);
    request.tools = Some(&tools);

    let dispatch = harness.call(Entry::GenerateStream, &request).await;

    assert!(matches!(
        dispatch.answer.expect("the stream entry call succeeds"),
        GenerateContentAnswer::Adapter(AdapterOutcome::Response(_))
    ));
    let seen = harness.executor.seen();
    assert_eq!(seen[0].0, Operation::Completion);
    assert!(seen[0].1.request.stream);
    assert_eq!(seen[0].1.request.tools, None);
    assert_eq!(
        dispatch
            .state
            .litellm_params
            .get("agenerate_content_stream"),
        Some(&DecisionJson::Bool(false))
    );
}

#[tokio::test]
async fn dispatch_async_stream_passes_tools_and_stream() {
    let mut harness = Harness::new();
    harness.executor = Scripted::answering(stream_output());
    let contents = contents();
    let kwargs = object("{}");
    let tools = json(TOOLS);
    let mut request = request(ADAPTER_MODEL, &contents, None, &kwargs);
    request.tools = Some(&tools);

    let dispatch = harness.call(Entry::AgenerateStream, &request).await;

    let GenerateContentAnswer::Adapter(AdapterOutcome::Stream(mut stream)) =
        dispatch.answer.expect("the stream entry call succeeds")
    else {
        panic!("a stream answer to a streaming call goes to the streaming translation");
    };
    let frame = stream
        .next()
        .await
        .expect("the stream answers a frame")
        .expect("the frame encodes");
    let text = String::from_utf8(frame).expect("the frame is UTF-8");
    assert!(text.starts_with("data: "));
    assert!(text.contains("hello there"));
    let seen = harness.executor.seen();
    assert_eq!(seen[0].0, Operation::Acompletion);
    assert!(seen[0].1.request.stream);
    assert_eq!(seen[0].1.request.tools.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        dispatch
            .state
            .litellm_params
            .get("agenerate_content_stream"),
        Some(&DecisionJson::Bool(true))
    );
}

#[tokio::test]
async fn dispatch_native_call_merges_native_fields_under_extra_body() {
    let mut harness = Harness::new();
    harness.config.top_fields = &["safetySettings"];
    let contents = contents();
    let kwargs = object(
        r#"{"safetySettings":[{"category":"HARM_CATEGORY_HATE_SPEECH","threshold":"BLOCK_NONE"}]}"#,
    );
    let extra_body = object(r#"{"labels":{"team":"a"}}"#);
    let mut request = request(NATIVE_MODEL, &contents, None, &kwargs);
    request.extra_body = Some(&extra_body);

    let dispatch = harness.call(Entry::Generate, &request).await;

    let answer = dispatch.answer.expect("the native call succeeds");
    let GenerateContentAnswer::Native(GenerateContentOutcome::Response(response)) = answer else {
        panic!("a native unary call answers the provider response");
    };
    assert_eq!(response, json(NATIVE_REPLY));
    let seen = harness.source.requests();
    let sent: Json = serde_json::from_slice(&seen[0].body).expect("the request body is JSON");
    let expected = json(
        r#"{"model":"gemini-2.5-flash","contents":[{"role":"user","parts":[{"text":"Hi"}]}],"generationConfig":{},"safetySettings":[{"category":"HARM_CATEGORY_HATE_SPEECH","threshold":"BLOCK_NONE"}],"labels":{"team":"a"}}"#,
    );
    assert_eq!(sent, expected);
    assert_eq!(harness.config.auth_calls()[0].model, "gemini-2.5-flash");
}

#[tokio::test]
async fn dispatch_native_call_defaults_a_missing_or_zero_timeout() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object("{}");
    let mut request = request(NATIVE_MODEL, &contents, None, &kwargs);

    let defaults = harness.call(Entry::Generate, &request).await;
    assert!(matches!(
        defaults.answer.expect("the native call succeeds"),
        GenerateContentAnswer::Native(GenerateContentOutcome::Response(_))
    ));
    request.timeout = Some(Duration::ZERO);
    let zero = harness.call(Entry::Generate, &request).await;
    assert!(matches!(
        zero.answer.expect("the zero timeout call succeeds"),
        GenerateContentAnswer::Native(GenerateContentOutcome::Response(_))
    ));
    request.timeout = Some(Duration::from_secs(30));
    let given = harness.call(Entry::Generate, &request).await;
    assert!(matches!(
        given.answer.expect("the given timeout call succeeds"),
        GenerateContentAnswer::Native(GenerateContentOutcome::Response(_))
    ));

    let seen = harness.source.requests();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].timeout, Some(Duration::from_secs(6000)));
    assert_eq!(seen[1].timeout, Some(Duration::from_secs(6000)));
    assert_eq!(seen[2].timeout, Some(Duration::from_secs(30)));
}

#[tokio::test]
async fn dispatch_native_call_prefers_the_camel_system_instruction() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object(
        r#"{"systemInstruction":{"parts":[{"text":"first"}]},"system_instruction":{"parts":[{"text":"second"}]}}"#,
    );
    let request = request(NATIVE_MODEL, &contents, None, &kwargs);

    harness
        .call(Entry::Generate, &request)
        .await
        .answer
        .expect("the native call succeeds");

    let seen = harness.source.requests();
    let sent: Json = serde_json::from_slice(&seen[0].body).expect("the request body is JSON");
    let Json::Object(fields) = &sent else {
        panic!("the request body is an object");
    };
    assert_eq!(
        fields.get("systemInstruction"),
        Some(&json(r#"{"parts":[{"text":"first"}]}"#))
    );
}

#[tokio::test]
async fn dispatch_native_call_falls_through_an_empty_system_instruction() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs =
        object(r#"{"systemInstruction":"","system_instruction":{"parts":[{"text":"second"}]}}"#);
    let request = request(NATIVE_MODEL, &contents, None, &kwargs);

    harness
        .call(Entry::Generate, &request)
        .await
        .answer
        .expect("the native call succeeds");

    let seen = harness.source.requests();
    let sent: Json = serde_json::from_slice(&seen[0].body).expect("the request body is JSON");
    let Json::Object(fields) = &sent else {
        panic!("the request body is an object");
    };
    assert_eq!(
        fields.get("systemInstruction"),
        Some(&json(r#"{"parts":[{"text":"second"}]}"#))
    );
}

#[tokio::test]
async fn dispatch_stream_entries_select_the_sync_and_async_native_handlers() {
    let mut harness = Harness::new();
    harness.source = Source::new(Wire::streaming(200, SSE));
    let contents = contents();
    let kwargs = object("{}");
    let request = request(NATIVE_MODEL, &contents, None, &kwargs);

    let sync_call = harness.call(Entry::GenerateStream, &request).await;
    assert!(matches!(
        sync_call.answer.expect("the sync stream starts"),
        GenerateContentAnswer::Native(GenerateContentOutcome::Stream(_))
    ));
    let async_call = harness.call(Entry::AgenerateStream, &request).await;
    assert!(matches!(
        async_call.answer.expect("the async stream starts"),
        GenerateContentAnswer::AsyncNative(AsyncGenerateContentOutcome::Stream(_))
    ));
    let seen = harness.source.requests();
    assert_eq!(seen.len(), 2);
    assert!(!seen[0].streaming);
    assert!(seen[1].streaming);
    assert_eq!(
        sync_call
            .state
            .litellm_params
            .get("agenerate_content_stream"),
        Some(&DecisionJson::Bool(false))
    );
    assert_eq!(
        async_call
            .state
            .litellm_params
            .get("agenerate_content_stream"),
        Some(&DecisionJson::Bool(true))
    );
}

#[test]
fn mock_response_has_litellm_shape_and_default_text() {
    let answer = mock_generate_content_response(None);
    assert_eq!(
        answer,
        json(&MOCK_TEMPLATE.replace("TEXT", DEFAULT_MOCK_TEXT))
    );
}

#[test]
fn mock_response_uses_the_custom_text() {
    let answer = mock_generate_content_response(Some("custom text"));
    assert_eq!(answer, json(&MOCK_TEMPLATE.replace("TEXT", "custom text")));
}

#[tokio::test]
async fn mock_generate_content_returns_the_mock_before_any_setup() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object(r#"{"mock_response":"mocked answer"}"#);
    // an unresolvable model proves setup never ran
    let request = request("missing-model", &contents, None, &kwargs);

    let dispatch = harness.call(Entry::Generate, &request).await;

    let answer = dispatch.answer.expect("the mock answers before setup");
    let GenerateContentAnswer::Mock(value) = answer else {
        panic!("a string mock answers the mock object");
    };
    assert_eq!(value, json(&MOCK_TEMPLATE.replace("TEXT", MOCK_TEXT)));
    assert_eq!(harness.source.requests().len(), 0);
    assert_eq!(harness.executor.seen().len(), 0);
}

#[tokio::test]
async fn mock_agenerate_content_returns_the_mock_before_any_setup() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object(r#"{"mock_response":"mocked answer"}"#);
    let request = request(NATIVE_MODEL, &contents, None, &kwargs);

    let dispatch = harness.call(Entry::Agenerate, &request).await;

    let answer = dispatch.answer.expect("the mock answers before setup");
    let GenerateContentAnswer::Mock(value) = answer else {
        panic!("a string mock answers the mock object");
    };
    assert_eq!(value, json(&MOCK_TEMPLATE.replace("TEXT", MOCK_TEXT)));
    assert_eq!(harness.source.requests().len(), 0);
    assert_eq!(
        dispatch.state.litellm_params.get("agenerate_content"),
        Some(&DecisionJson::Bool(true))
    );
}

#[tokio::test]
async fn mock_stream_entry_without_stream_flag_fails_and_with_it_proceeds() {
    let harness = Harness::new();
    let contents = contents();
    let without = object(r#"{"mock_response":"mocked answer"}"#);
    let with = object(r#"{"mock_response":"mocked answer","stream":true}"#);

    let failed = harness
        .call(
            Entry::GenerateStream,
            &request(NATIVE_MODEL, &contents, None, &without),
        )
        .await;
    let Err(failure) = failed.answer else {
        panic!("the setup guard refuses a mock");
    };
    assert!(matches!(
        failure.error,
        GenerateContentError::Setup(GenAiSetupError::MockResponseHandledByCaller)
    ));
    assert_eq!(failure.class.metadata().status_code, 500_u16);

    let proceeded = harness
        .call(
            Entry::GenerateStream,
            &request(NATIVE_MODEL, &contents, None, &with),
        )
        .await;
    assert!(matches!(
        proceeded
            .answer
            .expect("a stream flag lets the call reach the provider"),
        GenerateContentAnswer::Native(GenerateContentOutcome::Stream(_))
    ));
    assert_eq!(harness.source.requests().len(), 1);
}

#[tokio::test]
async fn error_bad_field_type_maps_to_a_named_bad_request() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object(r#"{"max_retries":"many"}"#);
    let request = request(ADAPTER_MODEL, &contents, Some("openai"), &kwargs);

    let dispatch = harness.call(Entry::Generate, &request).await;

    let Err(failure) = dispatch.answer else {
        panic!("the kwargs field type fails");
    };
    let GenerateContentError::Fields(field) = &failure.error else {
        panic!("the entry point reports the field itself");
    };
    assert_eq!(field.field, "max_retries");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a named field failure is a 400");
    };
    assert_eq!(details.body_param.as_deref(), Some("max_retries"));
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(ADAPTER_MODEL));
    assert_eq!(details.status.0, 400_u16);
}

#[tokio::test]
async fn error_provider_failure_keeps_the_status_litellm_maps() {
    let mut harness = Harness::new();
    harness.source = Source::new(Wire::answering(429, ERROR_BODY));
    let contents = contents();
    let kwargs = object("{}");
    let request = request(NATIVE_MODEL, &contents, None, &kwargs);

    let dispatch = harness.call(Entry::Generate, &request).await;

    let Err(failure) = dispatch.answer else {
        panic!("the provider answers an error");
    };
    assert!(matches!(
        &failure.error,
        GenerateContentError::Native(GenAiHttpError::Provider(_))
    ));
    assert!(matches!(&failure.class, ErrorClass::RateLimit(_)));
    assert_eq!(failure.class.metadata().status_code, 429_u16);
    let errors = harness.config.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].1, 429_u16);
}

#[tokio::test]
async fn error_stream_entry_without_provider_resolves_from_the_model_only() {
    let harness = Harness::new();
    let contents = contents();
    let kwargs = object("{}");
    let request = request("missing-model", &contents, None, &kwargs);

    let dispatch = harness.call(Entry::GenerateStream, &request).await;

    let Err(failure) = dispatch.answer else {
        panic!("the setup cannot resolve the provider");
    };
    let GenerateContentError::Setup(GenAiSetupError::Resolution(
        ResolutionError::MissingProvider { model },
    )) = &failure.error
    else {
        panic!("the stream entry resolves the provider itself");
    };
    assert_eq!(model, "missing-model");
    assert_eq!(failure.class.metadata().status_code, 400_u16);
    assert_eq!(failure.class.metadata().provider.as_deref(), Some(""));
}

#[tokio::test]
async fn error_adapter_failure_maps_to_an_api_connection_error() {
    let mut harness = Harness::new();
    harness.executor = Scripted::failing("boom");
    let contents = contents();
    let kwargs = object("{}");
    let request = request(ADAPTER_MODEL, &contents, Some("openai"), &kwargs);

    let dispatch = harness.call(Entry::Generate, &request).await;

    let Err(failure) = dispatch.answer else {
        panic!("the completion fails");
    };
    let GenerateContentError::Adapter(ChatCallError::Completion { operation, .. }) = &failure.error
    else {
        panic!("the adapter reports the completion failure");
    };
    assert_eq!(*operation, Operation::Completion);
    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(failure.class.metadata().status_code, 500_u16);
    assert!(
        failure
            .to_string()
            .contains("Error calling litellm.completion for generate_content: boom")
    );
}
