#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use http::HeaderMap;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use secrecy::SecretString;
use url::Url;

use super::custom::{self, CustomChatMethod, custom_chat_llm_router, decode_custom_output};
use super::error::LegacyError;
use super::{Legacy, aleph, proxy};
use crate::operation::{
    ChatCompletion, ChatOperation, EmbeddingOperation, OperationError, OperationRequest,
    OperationResult,
};
use crate::prepared::Upstream;
use crate::prepared::deployment::Deployment;
use crate::provider::Adapter as _;

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn deployment(model: &str, api_base: &str, api_key: Option<&str>) -> Deployment {
    let endpoint = format!("{}/chat/completions", api_base.trim_end_matches('/'));
    Deployment {
        model: String::from(model),
        api_base: String::from(api_base),
        endpoint: Url::parse(&endpoint).unwrap(),
        api_key: api_key.map(|key| SecretString::from(key.to_owned())),
        authorization: None,
        organization: None,
        timeout: Duration::from_secs(30),
        max_retries: None,
        drop_params: None,
        provider_affinity_header: None,
    }
}

fn chat_operation(body: &[u8], streamed: bool) -> OperationRequest {
    let chat = ChatOperation {
        model: String::from("client-alias"),
        request: ChatRequest::parse(body).unwrap(),
        control: ControlOptions::default(),
    };
    if streamed {
        OperationRequest::Stream(Box::new(chat))
    } else {
        OperationRequest::Chat(Box::new(chat))
    }
}

fn buffered(status: u16, body: &[u8]) -> Upstream {
    Upstream::Buffered(UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.to_vec(),
    })
}

fn object(body: &[u8]) -> JsonValue {
    let parsed: JsonValue = serde_json::from_slice(body).unwrap();
    let JsonValue::Object(fields) = parsed else {
        panic!("the request body is not an object");
    };
    JsonValue::Object(fields)
}

fn field<'a>(fields: &'a JsonValue, name: &str) -> &'a JsonValue {
    let JsonValue::Object(entries) = fields else {
        panic!("the request body is not an object");
    };
    entries
        .get(name)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn header_text(headers: &HeaderMap, name: http::header::HeaderName) -> String {
    let value = headers.get(name).unwrap();
    value.to_str().unwrap().to_owned()
}

#[test]
fn the_three_legacy_slugs_name_their_deployments() {
    assert_eq!(Legacy::LitellmProxy.slug(), "litellm_proxy");
    assert_eq!(Legacy::AlephAlpha.slug(), "aleph_alpha");
    assert_eq!(Legacy::Custom.slug(), "custom");
    assert_eq!(Legacy::default(), Legacy::LitellmProxy);
}

#[test]
fn the_custom_method_table_names_every_selector_answer() {
    assert_eq!(
        custom_chat_llm_router(true, Some(true)),
        CustomChatMethod::Astreaming
    );
    assert_eq!(
        custom_chat_llm_router(true, Some(false)),
        CustomChatMethod::Acompletion
    );
    assert_eq!(
        custom_chat_llm_router(true, None),
        CustomChatMethod::Acompletion
    );
    assert_eq!(
        custom_chat_llm_router(false, Some(true)),
        CustomChatMethod::Streaming
    );
    assert_eq!(
        custom_chat_llm_router(false, Some(false)),
        CustomChatMethod::Completion
    );
    assert_eq!(
        custom_chat_llm_router(false, None),
        CustomChatMethod::Completion
    );
    assert_eq!(CustomChatMethod::Completion.name(), "completion");
    assert_eq!(CustomChatMethod::Streaming.name(), "streaming");
    assert_eq!(CustomChatMethod::Acompletion.name(), "acompletion");
    assert_eq!(CustomChatMethod::Astreaming.name(), "astreaming");
}

#[test]
fn the_proxy_body_keeps_the_nested_model_and_the_supported_params() {
    let request = chat_operation(
        br#"{"model":"alias","messages":[{"role":"system","content":"You are terse."},{"role":"user","content":"hi"}],"temperature":0.2,"top_p":0.9,"max_tokens":20,"n":1,"seed":7,"stop":["\n"],"stream":false}"#,
        false,
    );
    let deployment = deployment(
        "gemini/gemini-1.5-pro",
        "https://proxy.example",
        Some("test-key"),
    );
    let call = proxy::prepare(&request, &deployment, &catalog()).unwrap();
    let body = object(&call.body);
    assert_eq!(
        field(&body, "model"),
        &JsonValue::Text(String::from("gemini/gemini-1.5-pro"))
    );
    assert_eq!(
        field(&body, "messages"),
        &JsonValue::Array(vec![
            JsonValue::Object(indexmap::IndexMap::from([
                (
                    String::from("role"),
                    JsonValue::Text(String::from("system"))
                ),
                (
                    String::from("content"),
                    JsonValue::Text(String::from("You are terse."))
                ),
            ])),
            JsonValue::Object(indexmap::IndexMap::from([
                (String::from("role"), JsonValue::Text(String::from("user"))),
                (String::from("content"), JsonValue::Text(String::from("hi"))),
            ])),
        ])
    );
    assert_eq!(
        field(&body, "temperature"),
        &JsonValue::Number(serde_json::Number::from_f64(0.2).unwrap())
    );
    assert_eq!(
        field(&body, "max_tokens"),
        &JsonValue::Number(serde_json::Number::from(20_u64))
    );
    assert_eq!(
        field(&body, "seed"),
        &JsonValue::Number(serde_json::Number::from(7_u64))
    );
    let JsonValue::Object(entries) = &body else {
        panic!();
    };
    assert!(
        !entries.contains_key("stream"),
        "a buffered call sends no stream flag"
    );
    assert_eq!(call.url.as_str(), "https://proxy.example/chat/completions");
    assert_eq!(header_text(&call.headers, AUTHORIZATION), "Bearer test-key");
    assert_eq!(header_text(&call.headers, ACCEPT), "application/json");
}

#[test]
fn the_proxy_sends_the_configured_organization_header() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let mut deployment = deployment("gpt-4o", "https://proxy.example", Some("test-key"));
    deployment.organization = Some(String::from("org-1"));
    let call = proxy::prepare(&request, &deployment, &catalog()).unwrap();
    let organization = http::header::HeaderName::from_static("openai-organization");
    assert_eq!(header_text(&call.headers, organization), "org-1");
}

#[test]
fn the_proxy_stream_asks_for_usage_only_from_an_openai_backed_base() {
    let request = chat_operation(
        br#"{"messages":[{"role":"user","content":"hi"}],"stream":true}"#,
        true,
    );
    let proxy_base = deployment(
        "gemini/gemini-1.5-pro",
        "https://proxy.example",
        Some("test-key"),
    );
    let call = proxy::prepare(&request, &proxy_base, &catalog()).unwrap();
    let body = object(&call.body);
    assert_eq!(field(&body, "stream"), &JsonValue::Bool(true));
    let JsonValue::Object(entries) = &body else {
        panic!();
    };
    assert!(!entries.contains_key("stream_options"));
    assert_eq!(header_text(&call.headers, ACCEPT), "text/event-stream");

    let openai_base = deployment("gpt-4o", "https://api.openai.com/v1", Some("test-key"));
    let call = proxy::prepare(&request, &openai_base, &catalog()).unwrap();
    let body = object(&call.body);
    let JsonValue::Object(usage) = field(&body, "stream_options") else {
        panic!("stream_options is not an object");
    };
    assert_eq!(usage.get("include_usage"), Some(&JsonValue::Bool(true)));
}

#[test]
fn the_proxy_keeps_the_callers_stream_options() {
    let request = chat_operation(
        br#"{"messages":[{"role":"user","content":"hi"}],"stream":true,"stream_options":{"include_usage":false}}"#,
        true,
    );
    let deployment = deployment("gpt-4o", "https://api.openai.com/v1", Some("test-key"));
    let call = proxy::prepare(&request, &deployment, &catalog()).unwrap();
    let body = object(&call.body);
    let JsonValue::Object(usage) = field(&body, "stream_options") else {
        panic!("stream_options is not an object");
    };
    assert_eq!(usage.get("include_usage"), Some(&JsonValue::Bool(false)));
}

#[test]
fn a_proxy_call_without_a_key_is_refused() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let deployment = deployment("gemini/gemini-1.5-pro", "https://proxy.example", None);
    let refused = proxy::prepare(&request, &deployment, &catalog()).unwrap_err();
    assert!(matches!(refused, OperationError::MissingApiKey));
}

#[test]
fn a_proxy_call_uses_the_url_userinfo_credential() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let mut deployment = deployment("gpt-4o", "https://proxy.example", None);
    deployment.authorization = Some(SecretString::from(String::from("Basic dXNlcjpwYXNz")));
    let call = proxy::prepare(&request, &deployment, &catalog()).unwrap();
    assert_eq!(
        header_text(&call.headers, AUTHORIZATION),
        "Basic dXNlcjpwYXNz"
    );
}

#[test]
fn the_proxy_reply_is_stamped_with_the_client_model() {
    let request = chat_operation(
        br#"{"model":"client-alias","messages":[{"role":"user","content":"hi"}]}"#,
        false,
    );
    let deployment = deployment(
        "gemini/gemini-1.5-pro",
        "https://proxy.example",
        Some("test-key"),
    );
    let call = proxy::prepare(&request, &deployment, &catalog()).unwrap();
    let reply = br#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream-model","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
    let result = proxy::decode(&call, buffered(200, reply)).unwrap();
    let OperationResult::Chat(ChatCompletion::OpenAi(response)) = result else {
        panic!("the proxy reply is not an OpenAI chat completion");
    };
    assert_eq!(response.model, "client-alias");
    assert_eq!(response.choices[0].message.content.as_deref(), Some("hi"));
}

#[test]
fn the_proxy_keeps_the_provider_error_body() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let deployment = deployment(
        "gemini/gemini-1.5-pro",
        "https://proxy.example",
        Some("test-key"),
    );
    let call = proxy::prepare(&request, &deployment, &catalog()).unwrap();
    let failed = proxy::decode(
        &call,
        buffered(429, br#"{"error":{"message":"slow down"}}"#),
    )
    .unwrap_err();
    let OperationError::Provider { response } = failed else {
        panic!("a provider status is not a provider error");
    };
    assert_eq!(response.status, 429);
    assert_eq!(response.body, br#"{"error":{"message":"slow down"}}"#);
}

#[test]
fn an_unparsable_reply_is_a_decode_error() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let deployment = deployment(
        "gemini/gemini-1.5-pro",
        "https://proxy.example",
        Some("test-key"),
    );
    let call = proxy::prepare(&request, &deployment, &catalog()).unwrap();
    let refused = proxy::decode(&call, buffered(200, b"not json")).unwrap_err();
    assert!(matches!(
        refused,
        OperationError::Decode {
            operation: "chat",
            ..
        }
    ));
}

#[test]
fn the_proxy_embedding_body_drops_an_absent_encoding_format() {
    let operation = embedding_operation(br#"{"input":["hello"]}"#);
    let deployment = deployment("emb-model", "https://proxy.example", Some("test-key"));
    let call = proxy::prepare(&operation, &deployment, &catalog()).unwrap();
    let body = object(&call.body);
    assert_eq!(
        field(&body, "model"),
        &JsonValue::Text(String::from("emb-model"))
    );
    assert_eq!(
        field(&body, "input"),
        &JsonValue::Array(vec![JsonValue::Text(String::from("hello"))])
    );
    let JsonValue::Object(entries) = &body else {
        panic!();
    };
    assert!(!entries.contains_key("encoding_format"));
    assert_eq!(call.url.as_str(), "https://proxy.example/embeddings");
    assert_eq!(header_text(&call.headers, AUTHORIZATION), "Bearer test-key");
    assert_eq!(header_text(&call.headers, CONTENT_TYPE), "application/json");
}

#[test]
fn the_proxy_embedding_body_keeps_the_requested_encoding_format() {
    let operation = embedding_operation(br#"{"input":"hello","encoding_format":"base64"}"#);
    let deployment = deployment("emb-model", "https://proxy.example", Some("test-key"));
    let call = proxy::prepare(&operation, &deployment, &catalog()).unwrap();
    let body = object(&call.body);
    assert_eq!(
        field(&body, "encoding_format"),
        &JsonValue::Text(String::from("base64"))
    );
}

#[test]
fn the_proxy_embedding_answer_carries_the_requested_model() {
    let operation = embedding_operation(br#"{"model":"client-alias","input":"hello"}"#);
    let deployment = deployment(
        "text-embedding-3-small",
        "https://proxy.example",
        Some("test-key"),
    );
    let call = proxy::prepare(&operation, &deployment, &catalog()).unwrap();
    let reply = br#"{"object":"list","data":[{"object":"embedding","embedding":[0.1],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":1,"total_tokens":1}}"#;
    let OperationResult::Embedding(decoded) = proxy::decode(&call, buffered(200, reply)).unwrap()
    else {
        panic!("an embedding reply is not an embedding result");
    };
    assert_eq!(decoded.model, "client-alias");
}

fn embedding_operation(body: &[u8]) -> OperationRequest {
    OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: String::from("emb-model"),
        request: EmbeddingRequest::parse(body).unwrap(),
    }))
}

#[test]
fn the_aleph_prompt_labels_control_roles() {
    let request = chat_operation(
        br#"{"messages":[{"role":"system","content":"You are terse."},{"role":"user","content":"hi"},{"role":"assistant","content":"ok"}]}"#,
        false,
    );
    let deployment = deployment(
        "luminous-base-control",
        "https://api.aleph-alpha.com/complete",
        Some("aleph-key"),
    );
    let call = aleph::prepare(&request, &deployment).unwrap();
    assert_eq!(
        call.request.prompt,
        "###Instruction: You are terse.###Input: hi###Response: ok"
    );
    assert_eq!(
        call.request
            .body
            .get("maximum_tokens")
            .and_then(|value| value.as_u64()),
        Some(4096_u64)
    );
    assert_eq!(
        header_text(&call.request.headers, AUTHORIZATION),
        "Bearer aleph-key"
    );
    assert_eq!(
        header_text(&call.request.headers, CONTENT_TYPE),
        "application/json"
    );
    assert_eq!(
        call.request.url.as_str(),
        "https://api.aleph-alpha.com/complete"
    );
}

#[test]
fn the_aleph_prompt_joins_messages_without_labels_for_other_models() {
    let request = chat_operation(
        br#"{"messages":[{"role":"user","content":"hello"},{"role":"assistant","content":"there"}]}"#,
        false,
    );
    let deployment = deployment(
        "luminous-base",
        "https://api.aleph-alpha.com/complete",
        Some("aleph-key"),
    );
    let call = aleph::prepare(&request, &deployment).unwrap();
    assert_eq!(call.request.prompt, "hello there");
}

#[test]
fn the_aleph_body_carries_the_call_parameters() {
    let request = chat_operation(
        br#"{"messages":[{"role":"user","content":"hi"}],"max_completion_tokens":32,"temperature":0.5,"top_p":0.8,"top_k":7}"#,
        false,
    );
    let deployment = deployment(
        "luminous-base",
        "https://api.aleph-alpha.com/complete",
        Some("aleph-key"),
    );
    let call = aleph::prepare(&request, &deployment).unwrap();
    let body = &call.request.body;
    // LiteLLM forwards max_completion_tokens as max_tokens for a provider without that parameter
    assert_eq!(
        body.get("max_tokens").and_then(|value| value.as_u64()),
        Some(32_u64)
    );
    assert!(body.get("max_completion_tokens").is_none());
    assert_eq!(
        body.get("temperature").and_then(|value| value.as_f64()),
        Some(0.5_f64)
    );
    assert_eq!(
        body.get("top_p").and_then(|value| value.as_f64()),
        Some(0.8_f64)
    );
    assert_eq!(
        body.get("top_k").and_then(|value| value.as_u64()),
        Some(7_u64)
    );
}

#[test]
fn the_aleph_reply_becomes_the_synthetic_chat_completion() {
    let request = chat_operation(
        br#"{"model":"client-alias","messages":[{"role":"user","content":"The capital of France is"}]}"#,
        false,
    );
    let deployment = deployment(
        "luminous-base",
        "https://api.aleph-alpha.com/complete",
        Some("aleph-key"),
    );
    let call = aleph::prepare(&request, &deployment).unwrap();
    let reply = br#"{"completions":[{"completion":"Paris.\n","finish_reason":"stop"}]}"#;
    let result = aleph::decode(&call, buffered(200, reply)).unwrap();
    let OperationResult::Chat(ChatCompletion::AiohttpOpenAi(response)) = result else {
        panic!("the aleph reply is not the synthetic chat shape");
    };
    assert_eq!(response.model.as_deref(), Some("client-alias"));
    assert_eq!(response.choices[0].index, 1);
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("Paris.\n")
    );
    assert_eq!(
        response.choices[0].message.role.as_deref(),
        Some("assistant")
    );
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
    assert!(response.usage.prompt_tokens > 0);
    assert!(response.usage.completion_tokens > 0);
    assert_eq!(
        response.usage.total_tokens,
        response.usage.prompt_tokens + response.usage.completion_tokens
    );
}

#[test]
fn an_aleph_error_body_keeps_its_provider_status() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let deployment = deployment(
        "luminous-base",
        "https://api.aleph-alpha.com/complete",
        Some("aleph-key"),
    );
    let call = aleph::prepare(&request, &deployment).unwrap();
    let refused = aleph::decode(&call, buffered(200, br#"{"error":"boom"}"#)).unwrap_err();
    let OperationError::Provider { response } = refused else {
        panic!("an error body is a provider error");
    };
    assert_eq!(response.body, b"boom");
}

#[test]
fn the_aleph_stream_is_refused() {
    let request = chat_operation(
        br#"{"messages":[{"role":"user","content":"hi"}],"stream":true}"#,
        true,
    );
    let deployment = deployment(
        "luminous-base",
        "https://api.aleph-alpha.com/complete",
        Some("aleph-key"),
    );
    let refused = aleph::prepare(&request, &deployment).unwrap_err();
    let OperationError::UnsupportedOperation {
        provider,
        operation,
    } = refused
    else {
        panic!("a stream is not an unsupported operation");
    };
    assert_eq!(provider, "aleph_alpha");
    assert_eq!(operation, "stream");
}

#[test]
fn the_aleph_embedding_is_refused() {
    let operation = embedding_operation(br#"{"input":"hello"}"#);
    let deployment = deployment(
        "luminous-base",
        "https://api.aleph-alpha.com/complete",
        Some("aleph-key"),
    );
    let refused = aleph::prepare(&operation, &deployment).unwrap_err();
    assert!(matches!(
        refused,
        OperationError::UnsupportedOperation { .. }
    ));
}

#[test]
fn the_custom_body_carries_every_parameter() {
    let request = chat_operation(
        br#"{"messages":[{"role":"system","content":"be terse"},{"role":"user","content":"hi"}],"max_tokens":16,"temperature":0.2,"top_p":0.9,"top_k":40}"#,
        false,
    );
    let deployment = deployment(
        "custom-model",
        "http://10.0.0.1:8080/generate",
        Some("custom-key"),
    );
    let call = custom::prepare(&request, &deployment).unwrap();
    assert_eq!(call.url.as_str(), "http://10.0.0.1:8080/generate");
    assert!(call.headers.get(AUTHORIZATION).is_none());
    assert_eq!(header_text(&call.headers, CONTENT_TYPE), "application/json");
    let body = object(&call.body);
    assert_eq!(
        field(&body, "model"),
        &JsonValue::Text(String::from("custom-model"))
    );
    assert_eq!(
        field(&body, "params"),
        &object(
            br#"{"prompt":["be terse hi"],"max_tokens":16,"temperature":0.2,"top_p":0.9,"top_k":40}"#
        )
    );
}

#[test]
fn the_custom_body_sends_null_parameters_that_were_not_set() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let deployment = deployment("custom-model", "http://10.0.0.1:8080/generate", None);
    let call = custom::prepare(&request, &deployment).unwrap();
    let body = object(&call.body);
    assert_eq!(
        field(&body, "params"),
        &object(
            br#"{"prompt":["hi"],"max_tokens":null,"temperature":null,"top_p":null,"top_k":null}"#
        )
    );
}

#[test]
fn the_custom_extra_body_expands_last() {
    let request = chat_operation(
        br#"{"messages":[{"role":"user","content":"hi"}],"extra_body":{"model":"other-model","params":{"prompt":["x"],"max_tokens":1}}}"#,
        false,
    );
    let deployment = deployment("custom-model", "http://10.0.0.1:8080/generate", None);
    let call = custom::prepare(&request, &deployment).unwrap();
    let body = object(&call.body);
    assert_eq!(
        field(&body, "model"),
        &JsonValue::Text(String::from("other-model"))
    );
    assert_eq!(
        field(&body, "params"),
        &object(br#"{"prompt":["x"],"max_tokens":1}"#)
    );
}

#[test]
fn a_non_object_extra_body_is_refused() {
    let request = chat_operation(
        br#"{"messages":[{"role":"user","content":"hi"}],"extra_body":"nope"}"#,
        false,
    );
    let deployment = deployment("custom-model", "http://10.0.0.1:8080/generate", None);
    let refused = custom::prepare(&request, &deployment).unwrap_err();
    assert!(matches!(
        refused,
        OperationError::Decode {
            operation: "chat",
            ..
        }
    ));
}

#[test]
fn a_message_without_text_content_is_refused() {
    let request = chat_operation(
        br#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]}"#,
        false,
    );
    let deployment = deployment("custom-model", "http://10.0.0.1:8080/generate", None);
    let refused = custom::prepare(&request, &deployment).unwrap_err();
    assert!(matches!(
        refused,
        OperationError::Decode {
            operation: "chat",
            ..
        }
    ));
}

#[test]
fn a_non_url_custom_base_is_refused() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let mut deployment = deployment("custom-model", "http://10.0.0.1:8080/generate", None);
    deployment.api_base = String::from("not a url");
    let refused = custom::prepare(&request, &deployment).unwrap_err();
    assert!(matches!(
        refused,
        OperationError::Decode {
            operation: "chat",
            ..
        }
    ));
}

#[test]
fn decode_custom_output_reads_the_first_text_output() {
    let body = br#"{"data":[{"prompt":"The capital of France is P","output":["The capital of France is PARIS.","second"]}],"message":"ok"}"#;
    assert_eq!(
        decode_custom_output(body).unwrap(),
        "The capital of France is PARIS."
    );
    let empty = br#"{"data":[{"output":[]}]}"#;
    assert!(matches!(
        decode_custom_output(empty),
        Err(LegacyError::CustomOutput)
    ));
    let number = br#"{"data":[{"output":[7]}]}"#;
    assert!(matches!(
        decode_custom_output(number),
        Err(LegacyError::CustomOutput)
    ));
    let no_data = br#"{"output":["x"]}"#;
    assert!(matches!(
        decode_custom_output(no_data),
        Err(LegacyError::CustomOutput)
    ));
    let array_root = br#"[]"#;
    assert!(matches!(
        decode_custom_output(array_root),
        Err(LegacyError::CustomOutput)
    ));
    let invalid = b"\xff";
    assert!(matches!(
        decode_custom_output(invalid),
        Err(LegacyError::Reply { .. })
    ));
}

#[test]
fn the_custom_reply_becomes_the_synthetic_chat_completion() {
    let request = chat_operation(
        br#"{"model":"client-alias","messages":[{"role":"user","content":"hi"}]}"#,
        false,
    );
    let deployment = deployment("custom-model", "http://10.0.0.1:8080/generate", None);
    let call = custom::prepare(&request, &deployment).unwrap();
    let reply = br#"{"data":[{"output":["Paris."]}]}"#;
    let result = custom::decode(&call, buffered(200, reply)).unwrap();
    let OperationResult::Chat(ChatCompletion::AiohttpOpenAi(response)) = result else {
        panic!("the custom reply is not the synthetic chat shape");
    };
    assert_eq!(response.object.as_deref(), Some("chat.completion"));
    assert_eq!(response.model.as_deref(), Some("client-alias"));
    assert!(response.created.is_some());
    assert_eq!(response.choices[0].index, 0);
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("Paris.")
    );
    // Choices() defaults a missing finish_reason to "stop" before the handler fills the content
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        response.usage,
        betterlite_openai::chat::response::ChatUsage::default()
    );
}

#[test]
fn a_custom_error_status_is_a_provider_error() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let deployment = deployment("custom-model", "http://10.0.0.1:8080/generate", None);
    let call = custom::prepare(&request, &deployment).unwrap();
    let refused = custom::decode(&call, buffered(500, b"upstream exploded")).unwrap_err();
    let OperationError::Provider { response } = refused else {
        panic!("a provider status is not a provider error");
    };
    assert_eq!(response.status, 500);
}

#[test]
fn a_custom_stream_is_refused() {
    let request = chat_operation(
        br#"{"messages":[{"role":"user","content":"hi"}],"stream":true}"#,
        true,
    );
    let deployment = deployment("custom-model", "http://10.0.0.1:8080/generate", None);
    let refused = custom::prepare(&request, &deployment).unwrap_err();
    let OperationError::UnsupportedOperation {
        provider,
        operation,
    } = refused
    else {
        panic!("a stream is not an unsupported operation");
    };
    assert_eq!(provider, "custom");
    assert_eq!(operation, "stream");
}

#[tokio::test]
async fn a_legacy_call_that_cannot_connect_is_a_transport_error() {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let client = ProxiedClient::outbound(&Arc::new(policy)).unwrap();
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let bases = [
        (Legacy::LitellmProxy, "http://127.0.0.1:9"),
        (Legacy::AlephAlpha, "http://127.0.0.1:9/complete"),
        (Legacy::Custom, "http://127.0.0.1:9/generate"),
    ];
    for (legacy, base) in bases {
        let deployment = deployment("model-name", base, Some("test-key"));
        let prepared = legacy.prepare(&request, &deployment, &catalog()).unwrap();
        let refused = match legacy.execute(&client, &prepared, &deployment).await {
            Ok(_) => panic!("a closed port answered"),
            Err(error) => error,
        };
        assert!(
            matches!(refused, OperationError::Http { .. }),
            "{legacy:?} answered {refused}"
        );
    }
}

#[test]
fn the_legacy_adapter_wraps_each_call_as_a_legacy_prepared_request() {
    let request = chat_operation(br#"{"messages":[{"role":"user","content":"hi"}]}"#, false);
    let deployment = deployment("custom-model", "http://10.0.0.1:8080/generate", None);
    let prepared = Legacy::Custom
        .prepare(&request, &deployment, &catalog())
        .unwrap();
    assert!(matches!(
        prepared,
        crate::prepared::PreparedRequest::Legacy(_)
    ));
    let refused = Legacy::Custom
        .prepare(
            &OperationRequest::Legacy(crate::legacy_dispatch::LegacyRequest),
            &deployment,
            &catalog(),
        )
        .unwrap_err();
    assert!(matches!(
        refused,
        OperationError::UnsupportedOperation { .. }
    ));
}

// The fuzzer's seed files: every named shape decodes to its text or the error LiteLLM raises
#[test]
fn replays_custom_output() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/custom_output");
    let mut seen = 0_usize;
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let name = path.file_stem().unwrap().to_str().unwrap();
        match name {
            "first-output" => assert_eq!(
                decode_custom_output(&bytes).unwrap(),
                "The capital of France is PARIS."
            ),
            "multiple-data" => assert_eq!(decode_custom_output(&bytes).unwrap(), "a"),
            "empty-output" | "number-output" | "missing-data" | "data-not-array"
            | "output-not-array" | "empty-data" | "null-output" | "array-root" | "message-only" => {
                assert!(matches!(
                    decode_custom_output(&bytes),
                    Err(LegacyError::CustomOutput)
                ))
            }
            "invalid-json" => assert!(matches!(
                decode_custom_output(&bytes),
                Err(LegacyError::Reply { .. })
            )),
            other => panic!("unknown custom_output seed {other}"),
        }
        seen += 1_usize;
    }
    assert_eq!(seen, 12_usize, "{seen} corpus files");
}
