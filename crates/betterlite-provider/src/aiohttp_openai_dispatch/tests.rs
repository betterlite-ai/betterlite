#![cfg(test)]

use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;
use url::Url;

use super::{AiohttpOpenAi, DEFAULT_AIOHTTP_BASE, complete_url};
use crate::operation::{
    ChatCompletion, ChatOperation, EmbeddingOperation, OperationError, OperationRequest,
    OperationResult,
};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Adapter as _;

fn deployment(api_base: &str, api_key: Option<&str>) -> Deployment {
    Deployment {
        model: String::from("up"),
        api_base: api_base.to_owned(),
        endpoint: Url::parse(&complete_url(api_base)).unwrap(),
        api_key: api_key.map(SecretString::from),
        authorization: None,
        organization: Some(String::from("org-1")),
        timeout: Duration::from_secs(30),
        max_retries: None,
        drop_params: None,
        provider_affinity_header: None,
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn chat(stream: bool) -> OperationRequest {
    let operation = Box::new(ChatOperation {
        model: String::from("up"),
        request: ChatRequest::parse(br#"{"model":"g","messages":[]}"#).unwrap(),
        control: ControlOptions::default(),
    });
    if stream {
        OperationRequest::Stream(operation)
    } else {
        OperationRequest::Chat(operation)
    }
}

fn prepared(api_base: &str, api_key: Option<&str>, stream: bool) -> PreparedRequest {
    AiohttpOpenAi
        .prepare(&chat(stream), &deployment(api_base, api_key), &catalog())
        .unwrap()
}

fn buffered(status: u16, body: &str) -> Upstream {
    Upstream::Buffered(UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    })
}

const FULL_ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","system_fingerprint":"fp","service_tier":"auto","choices":[{"index":0,"message":{"role":"assistant","content":"hi","refusal":null},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;

fn bare_base_gets_the_chat_path() {
    assert_eq!(
        complete_url(DEFAULT_AIOHTTP_BASE),
        "https://api.openai.com/chat/completions"
    );
    assert_eq!(
        complete_url("https://host/v1/chat/completions"),
        "https://host/v1/chat/completions"
    );
    let PreparedRequest::Chat(chat) = prepared("https://host/v1", Some("k"), false) else {
        panic!("a buffered chat prepares the chat request");
    };
    assert_eq!(chat.http.url.as_str(), "https://host/v1/chat/completions");
}

fn bearer_never_falls_back_to_the_environment() {
    let PreparedRequest::Chat(keyed) = prepared("https://host", Some("configured-key"), false)
    else {
        panic!("a buffered chat prepares the chat request");
    };
    assert_eq!(
        keyed.http.headers.get("authorization").unwrap(),
        "Bearer configured-key"
    );
    assert!(keyed.http.headers.get("openai-organization").is_none());
    let PreparedRequest::Chat(keyless) = prepared("https://host", None, false) else {
        panic!("a buffered chat prepares the chat request");
    };
    assert_eq!(
        keyless.http.headers.get("authorization").unwrap(),
        "Bearer None"
    );
}

fn response_copies_only_the_projected_fields() {
    let prepared = prepared("https://host", Some("k"), false);
    let decoded = AiohttpOpenAi
        .decode(&prepared, buffered(200, FULL_ANSWER))
        .unwrap();
    let OperationResult::Chat(ChatCompletion::AiohttpOpenAi(completion)) = decoded else {
        panic!("the aiohttp projection decodes to its own completion");
    };
    let written: JsonValue =
        serde_json::from_slice(&serde_json::to_vec(&*completion).unwrap()).unwrap();
    let wanted: JsonValue = serde_json::from_str(
        r#"{"id":"c1","object":"chat.completion","created":7,"model":"g","system_fingerprint":"fp","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}}"#,
    )
    .unwrap();
    assert_eq!(written, wanted);
}

fn missing_finish_reason_is_stop_and_choice_nulls_are_dropped() {
    let prepared = prepared("https://host", Some("k"), false);
    let answer = r#"{"id":"c1","object":"chat.completion","created":7,"model":"m","choices":[{"index":0,"foo":null,"bar":1,"message":{"role":"assistant","content":null,"baz":null}}]}"#;
    let decoded = AiohttpOpenAi
        .decode(&prepared, buffered(200, answer))
        .unwrap();
    let OperationResult::Chat(ChatCompletion::AiohttpOpenAi(completion)) = decoded else {
        panic!("the aiohttp projection decodes to its own completion");
    };
    let written: JsonValue =
        serde_json::from_slice(&serde_json::to_vec(&*completion).unwrap()).unwrap();
    let wanted: JsonValue = serde_json::from_str(
        r#"{"id":"c1","object":"chat.completion","created":7,"model":"g","choices":[{"index":0,"bar":1,"message":{"role":"assistant","content":null},"finish_reason":"stop"}],"usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}}"#,
    )
    .unwrap();
    assert_eq!(written, wanted);
}

fn provider_error_status_is_a_provider_error() {
    let prepared = prepared("https://host", Some("k"), false);
    let Err(OperationError::Provider { response }) =
        AiohttpOpenAi.decode(&prepared, buffered(500, "{}"))
    else {
        panic!("a failing status is a provider error");
    };
    assert_eq!(response.status, 500);
}

fn stream_request_keeps_the_stream_variant() {
    assert!(matches!(
        prepared("https://host", Some("k"), true),
        PreparedRequest::Stream(_)
    ));
}

fn sent_body(given: &str, stream: bool) -> JsonValue {
    let operation = Box::new(ChatOperation {
        model: String::from("up"),
        request: ChatRequest::parse(given.as_bytes()).unwrap(),
        control: ControlOptions::default(),
    });
    let request = if stream {
        OperationRequest::Stream(operation)
    } else {
        OperationRequest::Chat(operation)
    };
    let (PreparedRequest::Chat(prepared) | PreparedRequest::Stream(prepared)) = AiohttpOpenAi
        .prepare(&request, &deployment("https://host", Some("k")), &catalog())
        .unwrap()
    else {
        panic!("a chat request prepares the chat request");
    };
    serde_json::from_slice(&prepared.http.body).unwrap()
}

fn only_a_buffered_body_gets_the_router_stream_false() {
    let buffered: JsonValue =
        serde_json::from_str(r#"{"model":"up","messages":[],"stream":false}"#).unwrap();
    assert_eq!(sent_body(r#"{"model":"g","messages":[]}"#, false), buffered);
    assert_eq!(
        sent_body(r#"{"model":"g","messages":[],"stream":false}"#, false),
        buffered
    );
    let streamed: JsonValue = serde_json::from_str(
        r#"{"model":"up","messages":[],"stream":true,"stream_options":{"include_usage":true}}"#,
    )
    .unwrap();
    assert_eq!(
        sent_body(r#"{"model":"g","messages":[],"stream":true}"#, true),
        streamed
    );
}

fn embedding_is_unsupported_before_http() {
    let embedding = OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: String::from("up"),
        request: serde_json::from_str(r#"{"input":"hello"}"#).unwrap(),
    }));
    let refused = AiohttpOpenAi
        .prepare(
            &embedding,
            &deployment("https://host", Some("k")),
            &catalog(),
        )
        .unwrap_err();
    assert!(matches!(
        refused,
        OperationError::UnsupportedOperation { .. }
    ));
}

fn decoded_answer(answer: &str) -> JsonValue {
    let prepared = prepared("https://host", Some("k"), false);
    let decoded = AiohttpOpenAi
        .decode(&prepared, buffered(200, answer))
        .unwrap();
    let OperationResult::Chat(ChatCompletion::AiohttpOpenAi(completion)) = decoded else {
        panic!("the aiohttp projection decodes to its own completion");
    };
    serde_json::from_slice(&serde_json::to_vec(&*completion).unwrap()).unwrap()
}

fn choices_answer(choices: &str) -> String {
    format!(
        r#"{{"id":"c1","object":"chat.completion","created":7,"model":"upstream","choices":[{choices}]}}"#
    )
}

fn choices_of(answer: &str) -> JsonValue {
    let JsonValue::Object(mut fields) = decoded_answer(answer) else {
        panic!("a completion is an object");
    };
    fields.shift_remove("choices").unwrap()
}

fn finish_reasons_are_mapped_and_the_native_one_is_kept() {
    let answer = choices_answer(
        r#"{"index":0,"message":{"role":"assistant","content":"a"},"finish_reason":"max_tokens"},
        {"index":1,"message":{"role":"assistant","content":"b"},"finish_reason":"made_up"},
        {"index":2,"message":{"role":"assistant","content":"c"},"finish_reason":"length"},
        {"index":3,"message":{"role":"assistant","content":"d"},"finish_reason":null}"#,
    );
    let wanted: JsonValue = serde_json::from_str(
        r#"[
        {"index":0,"message":{"role":"assistant","content":"a"},"finish_reason":"length","provider_specific_fields":{"native_finish_reason":"max_tokens"}},
        {"index":1,"message":{"role":"assistant","content":"b"},"finish_reason":"stop","provider_specific_fields":{"native_finish_reason":"made_up"}},
        {"index":2,"message":{"role":"assistant","content":"c"},"finish_reason":"length"},
        {"index":3,"message":{"role":"assistant","content":"d"},"finish_reason":"stop"}]"#,
    )
    .unwrap();
    assert_eq!(choices_of(&answer), wanted);
}

fn role_defaults_to_assistant_and_empty_tool_calls_are_dropped() {
    let answer = choices_answer(
        r#"{"index":0,"message":{"content":"x","tool_calls":[]},"finish_reason":"stop"},
        {"index":1,"message":{"role":"user","content":"y","tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{}"}}]},"finish_reason":"tool_calls"}"#,
    );
    let wanted: JsonValue = serde_json::from_str(
        r#"[
        {"index":0,"message":{"role":"assistant","content":"x"},"finish_reason":"stop"},
        {"index":1,"message":{"role":"user","content":"y","tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{}"}}]},"finish_reason":"tool_calls"}]"#,
    )
    .unwrap();
    assert_eq!(choices_of(&answer), wanted);
}

fn the_reply_names_the_model_the_client_asked_for() {
    let answer = choices_answer(r#"{"index":0,"message":{"content":"x"}}"#);
    let JsonValue::Object(fields) = decoded_answer(&answer) else {
        panic!("a completion is an object");
    };
    assert_eq!(
        fields.get("model"),
        Some(&JsonValue::Text(String::from("g")))
    );
}

fn max_completion_tokens_is_sent_as_max_tokens() {
    let operation = Box::new(ChatOperation {
        model: String::from("up"),
        request: ChatRequest::parse(br#"{"model":"g","messages":[],"max_completion_tokens":5}"#)
            .unwrap(),
        control: ControlOptions::default(),
    });
    let PreparedRequest::Chat(chat) = AiohttpOpenAi
        .prepare(
            &OperationRequest::Chat(operation),
            &deployment("https://host", Some("k")),
            &catalog(),
        )
        .unwrap()
    else {
        panic!("a buffered chat prepares the chat request");
    };
    let sent = String::from_utf8(chat.http.body).unwrap();
    assert!(sent.contains(r#""max_tokens":5"#));
    assert!(!sent.contains("max_completion_tokens"));
}

fn the_debug_text_hides_the_key() {
    let shown = format!(
        "{:?}",
        prepared("https://host", Some("secret-key-9"), false)
    );
    assert!(!shown.contains("secret-key-9"));
    assert!(shown.contains("Sensitive"));
}

fn an_absent_base_reaches_the_bare_openai_host() {
    let PreparedRequest::Chat(chat) = prepared(DEFAULT_AIOHTTP_BASE, None, false) else {
        panic!("a buffered chat prepares the chat request");
    };
    assert_eq!(
        chat.http.url.as_str(),
        "https://api.openai.com/chat/completions"
    );
}

#[rstest]
#[case::finish_reason_mapping(finish_reasons_are_mapped_and_the_native_one_is_kept)]
#[case::role_and_tool_calls(role_defaults_to_assistant_and_empty_tool_calls_are_dropped)]
#[case::client_model(the_reply_names_the_model_the_client_asked_for)]
#[case::max_completion_tokens(max_completion_tokens_is_sent_as_max_tokens)]
#[case::debug_hides_key(the_debug_text_hides_the_key)]
#[case::absent_base(an_absent_base_reaches_the_bare_openai_host)]
#[case::bare_base(bare_base_gets_the_chat_path)]
#[case::bearer_without_fallback(bearer_never_falls_back_to_the_environment)]
#[case::projection_zeroes_usage(response_copies_only_the_projected_fields)]
#[case::defaults_and_null_extras(missing_finish_reason_is_stop_and_choice_nulls_are_dropped)]
#[case::provider_error(provider_error_status_is_a_provider_error)]
#[case::stream_variant(stream_request_keeps_the_stream_variant)]
#[case::router_stream_false(only_a_buffered_body_gets_the_router_stream_false)]
#[case::embedding_unsupported(embedding_is_unsupported_before_http)]
fn provider_framework_tests_behavior(#[case] check: fn()) {
    check();
}
