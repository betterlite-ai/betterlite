#![cfg(test)]

use std::fs;
use std::future::{Future, ready};
use std::time::Duration;

use betterlite_models::decisions::Json as ModelJson;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};
use betterlite_prompts::arguments::json::Object;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use crate::oci_chat::config::{
    get_complete_url, get_error_class, get_supported_openai_params, get_vendor_from_model,
    map_openai_params, uses_max_completion_tokens, validate_environment,
};
use crate::oci_chat::payload::{OciChatRequest, transform_request};
use crate::oci_chat::response::{decode_chat_body, transform_response};
use crate::oci_chat::stream::{STREAMING_TIMEOUT, SseDecoder, StreamItem, check_stream_status};
use crate::oci_chat_wire::types::oci::OciVendor;
use crate::oci_common::credentials::OciCredentials;
use crate::oci_common::environment::OciEnvironment;
use crate::oci_common::{Clock, OciError};

struct FakePredicates {
    reasoning: bool,
}

impl CapabilityPredicates for FakePredicates {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        let supported = self.reasoning
            && key == "supports_reasoning"
            && request.custom_llm_provider == Some("oci");
        ready(supported)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = ModelJson> + Send {
        ready(ModelJson::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

struct FixedClock(u64);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(i64::try_from(self.0).unwrap(), 0).unwrap()
    }
}

fn message(role: Role, text: &str) -> ChatMessage {
    ChatMessage {
        role,
        content: Some(MessageContent::Text(text.to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

fn credentials() -> OciCredentials {
    OciCredentials {
        oci_region: String::from("us-ashburn-1"),
        oci_user: Some(String::from("user")),
        oci_fingerprint: Some(String::from("fp")),
        oci_tenancy: Some(String::from("tenancy")),
        oci_key: Some(SecretString::from("key")),
        oci_key_file: None,
        oci_compartment_id: Some(String::from("compartment")),
    }
}

fn params(value: &serde_json::Value) -> Object {
    serde_json::from_value(value.clone()).unwrap()
}

async fn payload(model: &str, messages: &[ChatMessage], optional: &serde_json::Value) -> OciError {
    transform_request(
        model,
        messages,
        &params(optional),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap_err()
}

#[test]
fn supported_params_lists_each_vendors_map() {
    assert_eq!(
        get_supported_openai_params("meta.llama-3.3-70b-instruct"),
        vec![
            "stream",
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "tools",
            "frequency_penalty",
            "logprobs",
            "logit_bias",
            "n",
            "presence_penalty",
            "reasoning_effort",
            "seed",
            "stop",
            "tool_choice",
            "top_p",
            "response_format",
        ]
    );
    assert_eq!(
        get_supported_openai_params("cohere.command-r"),
        vec![
            "stream",
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "tools",
            "frequency_penalty",
            "presence_penalty",
            "seed",
            "stop",
            "top_p",
            "response_format",
            "top_k",
        ]
    );
}

#[rstest]
#[case("cohere.command-r", OciVendor::Cohere)]
#[case("oci/cohere.command-r", OciVendor::Cohere)]
#[case("OCI/Cohere.command-r", OciVendor::Cohere)]
#[case("meta.llama-3.3-70b-instruct", OciVendor::Generic)]
#[case("oci/openai.gpt-5", OciVendor::Generic)]
#[case("", OciVendor::Generic)]
fn vendor_from_model_reads_the_prefix_and_first_segment(
    #[case] model: &str,
    #[case] vendor: OciVendor,
) {
    assert_eq!(get_vendor_from_model(model), vendor);
}

#[test]
fn map_params_renames_and_keeps_passthrough_keys() {
    let mapped = map_openai_params(
        &params(&json!({
            "max_tokens": 5,
            "stream": true,
            "response_format": {"type": "text"},
            "user": "abc",
        })),
        &params(&json!({})),
        "meta.llama-3.3-70b-instruct",
        false,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&mapped).unwrap(),
        json!({
            "maxTokens": 5,
            "isStream": true,
            "responseFormat": {"type": "text"},
            "response_format": {"type": "text"},
            "user": "abc",
        })
    );
}

#[test]
fn map_params_lets_optional_params_win() {
    let mapped = map_openai_params(
        &params(&json!({"temperature": 0.1, "seed": 1})),
        &params(&json!({"temperature": 0.9})),
        "meta.llama-3.3-70b-instruct",
        false,
    )
    .unwrap();
    let value = serde_json::to_value(&mapped).unwrap();
    assert_eq!(value["temperature"], json!(0.9));
    assert_eq!(value["seed"], json!(1));
}

#[test]
fn map_params_drops_or_rejects_unsupported() {
    let rejected = map_openai_params(
        &params(&json!({})),
        &params(&json!({"modalities": ["text"]})),
        "meta.llama-3.3-70b-instruct",
        false,
    )
    .unwrap_err();
    assert_eq!(rejected.status, 400);
    assert_eq!(
        rejected.message,
        "param `modalities` is not supported on OCI"
    );
    let dropped = map_openai_params(
        &params(&json!({"modalities": ["text"], "top_logprobs": 2})),
        &params(&json!({})),
        "meta.llama-3.3-70b-instruct",
        true,
    )
    .unwrap();
    assert!(dropped.is_empty());
}

#[test]
fn map_params_skips_max_retries_and_n_one() {
    let mapped = map_openai_params(
        &params(&json!({"max_retries": 3, "n": 1, "top_p": 0.5})),
        &params(&json!({})),
        "cohere.command-r",
        false,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(&mapped).unwrap(), json!({"topP": 0.5}));
    let rejected = map_openai_params(
        &params(&json!({"n": 2})),
        &params(&json!({})),
        "cohere.command-r",
        false,
    )
    .unwrap_err();
    assert_eq!(rejected.status, 400);
}

#[test]
fn map_params_applies_the_cohere_map() {
    let mapped = map_openai_params(
        &params(&json!({"stop": ["x"], "top_k": 4, "logit_bias": {"1": 2}})),
        &params(&json!({})),
        "cohere.command-r",
        false,
    )
    .unwrap_err();
    assert_eq!(mapped.message, "param `logit_bias` is not supported on OCI");
    let mapped = map_openai_params(
        &params(&json!({"stop": ["x"], "top_k": 4})),
        &params(&json!({})),
        "cohere.command-r",
        false,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&mapped).unwrap(),
        json!({"stopSequences": ["x"], "topK": 4})
    );
}

#[test]
fn validate_environment_checks_messages_then_credentials() {
    let environment = OciEnvironment::default();
    let empty =
        validate_environment(&mut HeaderMap::new(), &[], &credentials(), &environment).unwrap_err();
    assert_eq!(empty.status, 400);
    let mut incomplete = credentials();
    incomplete.oci_user = None;
    let missing = validate_environment(
        &mut HeaderMap::new(),
        &[message(Role::User, "hi")],
        &incomplete,
        &environment,
    )
    .unwrap_err();
    assert_eq!(missing.status, 401);
    assert!(
        missing
            .message
            .starts_with("Missing required parameters: oci_user")
    );
    let mut keyless = credentials();
    keyless.oci_key = None;
    let missing = validate_environment(
        &mut HeaderMap::new(),
        &[message(Role::User, "hi")],
        &keyless,
        &environment,
    )
    .unwrap_err();
    assert_eq!(missing.status, 401);
}

#[test]
fn validate_environment_adds_the_json_headers() {
    let mut headers = HeaderMap::new();
    validate_environment(
        &mut headers,
        &[message(Role::User, "hi")],
        &credentials(),
        &OciEnvironment::default(),
    )
    .unwrap();
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["user-agent"], "litellm/1.105.1");
}

#[test]
fn complete_url_uses_the_chat_action() {
    assert_eq!(
        get_complete_url(&credentials(), None).unwrap(),
        "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/chat"
    );
    assert_eq!(
        get_complete_url(&credentials(), Some("https://proxy.example")).unwrap(),
        "https://proxy.example/20231130/actions/chat"
    );
}

#[test]
fn get_error_class_keeps_status_and_message() {
    let error = get_error_class("boom", 418);
    assert_eq!(error.status, 418);
    assert_eq!(error.message, "boom");
}

#[tokio::test]
async fn request_builds_a_generic_on_demand_payload() {
    let payload = transform_request(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &params(&json!({})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(&payload).unwrap(),
        json!({
            "compartmentId": "compartment",
            "servingMode": {"servingType": "ON_DEMAND", "modelId": "meta.llama-3.3-70b-instruct"},
            "chatRequest": {
                "apiFormat": "GENERIC",
                "messages": [{"role": "USER", "content": [{"type": "TEXT", "text": "hi"}]}],
                "isStream": false,
                "maxTokens": 4096,
            },
        })
    );
}

#[tokio::test]
async fn request_builds_a_dedicated_payload_and_rejects_a_bad_mode() {
    let dedicated = transform_request(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &params(&json!({"oci_serving_mode": "DEDICATED", "oci_endpoint_id": "ocid1.endpoint"})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(&dedicated).unwrap();
    assert_eq!(
        value["servingMode"],
        json!({"servingType": "DEDICATED", "endpointId": "ocid1.endpoint"})
    );
    let fallback = transform_request(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &params(&json!({"oci_serving_mode": "DEDICATED"})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(&fallback).unwrap();
    assert_eq!(
        value["servingMode"],
        json!({"servingType": "DEDICATED", "endpointId": "meta.llama-3.3-70b-instruct"})
    );
    let bad = payload(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &json!({"oci_serving_mode": "dedicated"}),
    )
    .await;
    assert_eq!(bad.status, 400);
}

#[tokio::test]
async fn request_builds_a_cohere_payload_with_preamble_and_history() {
    let payload = transform_request(
        "cohere.command-r",
        &[
            message(Role::System, "be"),
            message(Role::System, "nice"),
            message(Role::User, "first"),
            message(Role::Assistant, "answer"),
            message(Role::User, "second"),
        ],
        &params(&json!({"temperature": 0.2})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    let OciChatRequest::Cohere(request) = &payload.chat_request else {
        panic!("a Cohere model builds a Cohere request");
    };
    assert_eq!(request.message, "second");
    assert_eq!(request.preamble_override.as_deref(), Some("be\nnice"));
    let history = request.chat_history.as_ref().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        serde_json::to_value(&payload).unwrap()["chatRequest"]["maxTokens"],
        json!(4096)
    );
    assert_eq!(
        serde_json::to_value(&payload).unwrap()["chatRequest"]["temperature"],
        json!(0.2)
    );
}

#[tokio::test]
async fn request_rejects_a_cohere_call_without_a_user_message() {
    let error = payload(
        "cohere.command-r",
        &[message(Role::System, "be")],
        &json!({}),
    )
    .await;
    assert_eq!(error.status, 400);
    assert!(error.message.contains("No user message found"));
}

#[tokio::test]
async fn request_requires_a_compartment_id() {
    let mut absent = credentials();
    absent.oci_compartment_id = None;
    let error = transform_request(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &params(&json!({})),
        &absent,
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert!(error.message.contains("oci_compartment_id is required"));
}

#[rstest]
#[case("openai.gpt-5", false, "maxCompletionTokens", 4096)]
#[case("oci/OpenAI.gpt-5", false, "maxCompletionTokens", 4096)]
#[case("openai.gpt-oss-120b", false, "maxTokens", 4096)]
#[case("cohere.command-r", true, "maxTokens", 4096)]
#[case("google.gemini-2.5-flash", true, "maxCompletionTokens", 4096)]
#[case("google.gemini-2.5-flash", false, "maxTokens", 4096)]
#[tokio::test]
async fn token_cap_follows_the_model(
    #[case] model: &str,
    #[case] reasoning: bool,
    #[case] key: &str,
    #[case] cap: i64,
) {
    let payload = transform_request(
        model,
        &[message(Role::User, "hi")],
        &params(&json!({})),
        &credentials(),
        &mut FakePredicates { reasoning },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value["chatRequest"][key], json!(cap));
}

#[tokio::test]
async fn token_cap_uses_the_first_token_key_present() {
    let explicit = transform_request(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &params(&json!({"max_tokens": 7})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(&explicit).unwrap()["chatRequest"]["maxTokens"],
        json!(7)
    );
    let completion = transform_request(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &params(&json!({"max_completion_tokens": 9})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(&completion).unwrap()["chatRequest"]["maxTokens"],
        json!(9)
    );
    let capped = transform_request(
        "openai.gpt-5",
        &[message(Role::User, "hi")],
        &params(&json!({"max_tokens": 7})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(&capped).unwrap();
    assert_eq!(value["chatRequest"]["maxCompletionTokens"], json!(7));
    assert!(value["chatRequest"].get("maxTokens").is_none());
}

#[tokio::test]
async fn reasoning_effort_is_upper_cased_and_disable_becomes_none() {
    for (given, expected) in [("low", "LOW"), ("MINIMAL", "MINIMAL"), ("disable", "NONE")] {
        let payload = transform_request(
            "meta.llama-3.3-70b-instruct",
            &[message(Role::User, "hi")],
            &params(&json!({"reasoning_effort": given})),
            &credentials(),
            &mut FakePredicates { reasoning: false },
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(&payload).unwrap()["chatRequest"]["reasoningEffort"],
            json!(expected)
        );
    }
}

#[tokio::test]
async fn tools_are_adapted_per_vendor() {
    let tools = json!([{
        "type": "function",
        "function": {
            "name": "lookup",
            "description": "find it",
            "parameters": {"type": "object", "properties": {"q": {"type": "string"}}},
        },
    }]);
    let generic = transform_request(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &params(&json!({"tools": tools})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(&generic).unwrap();
    assert_eq!(value["chatRequest"]["tools"][0]["type"], json!("FUNCTION"));
    assert_eq!(value["chatRequest"]["tools"][0]["name"], json!("lookup"));
    let cohere = transform_request(
        "cohere.command-r",
        &[message(Role::User, "hi")],
        &params(&json!({"tools": tools})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(&cohere).unwrap();
    assert_eq!(value["chatRequest"]["tools"][0]["name"], json!("lookup"));
    assert_eq!(
        value["chatRequest"]["tools"][0]["parameterDefinitions"]["q"]["type"],
        json!("str")
    );
}

#[rstest]
#[case(json!("Auto"), json!({"type": "AUTO"}))]
#[case(json!("none"), json!({"type": "NONE"}))]
#[case(json!("ANY"), json!({"type": "REQUIRED"}))]
#[case(json!("My_Function"), json!({"type": "FUNCTION", "name": "My_Function"}))]
#[case(json!({"type": "function", "function": {"name": "lookup"}}), json!({"type": "FUNCTION", "name": "lookup"}))]
#[case(json!({"type": "required"}), json!({"type": "REQUIRED"}))]
#[tokio::test]
async fn tool_choice_strings_and_objects(
    #[case] given: serde_json::Value,
    #[case] expected: serde_json::Value,
) {
    let payload = transform_request(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &params(&json!({"tool_choice": given})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(&payload).unwrap()["chatRequest"]["toolChoice"],
        expected
    );
}

#[rstest]
#[case(json!({"type": 5}))]
#[case(json!({"type": "function"}))]
#[case(json!({"type": "function", "name": ""}))]
#[case(json!({"type": "weird"}))]
#[case(json!([1, 2]))]
#[tokio::test]
async fn tool_choice_rejects_bad_shapes(#[case] given: serde_json::Value) {
    let error = payload(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &json!({"tool_choice": given}),
    )
    .await;
    assert_eq!(error.status, 400);
    assert!(error.message.starts_with("Invalid tool_choice for OCI"));
}

#[tokio::test]
async fn tool_choice_names_the_rejected_kind() {
    let error = payload(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &json!({"tool_choice": [1, 2]}),
    )
    .await;
    assert_eq!(
        error.message,
        "Invalid tool_choice for OCI: expected str or dict, got list"
    );
}

#[rstest]
#[case("meta.llama-3.3-70b-instruct", json!({"type": "text"}), json!({"type": "TEXT"}))]
#[case("meta.llama-3.3-70b-instruct", json!({"type": "json_object"}), json!({"type": "JSON_OBJECT"}))]
#[case("meta.llama-3.3-70b-instruct", json!({"type": "json"}), json!({"type": "JSON_OBJECT"}))]
#[case(
    "meta.llama-3.3-70b-instruct",
    json!({"type": "json_schema", "json_schema": {"name": "n", "description": "d", "schema": {"type": "object"}, "strict": true}}),
    json!({"type": "JSON_SCHEMA", "jsonSchema": {"name": "n", "description": "d", "schema": {"type": "object"}, "isStrict": true}})
)]
#[case(
    "meta.llama-3.3-70b-instruct",
    json!({"type": "json_schema", "json_schema": {"schema": {"type": "object"}}}),
    json!({"type": "JSON_SCHEMA", "jsonSchema": {"name": "response", "schema": {"type": "object"}}})
)]
#[case(
    "cohere.command-r",
    json!({"type": "json_schema", "json_schema": {"name": "n", "schema": {"type": "object"}, "strict": true}}),
    json!({"type": "JSON_OBJECT", "schema": {"type": "object"}})
)]
#[tokio::test]
async fn response_format_maps_per_vendor(
    #[case] model: &str,
    #[case] given: serde_json::Value,
    #[case] expected: serde_json::Value,
) {
    let payload = transform_request(
        model,
        &[message(Role::User, "hi")],
        &params(&json!({"response_format": given})),
        &credentials(),
        &mut FakePredicates { reasoning: false },
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(&payload).unwrap()["chatRequest"]["responseFormat"],
        expected
    );
}

#[tokio::test]
async fn response_format_rejects_missing_schema() {
    let error = payload(
        "meta.llama-3.3-70b-instruct",
        &[message(Role::User, "hi")],
        &json!({"response_format": {"type": "json_schema"}}),
    )
    .await;
    assert_eq!(error.status, 400);
    assert!(error.message.contains("requires a 'json_schema' object"));
}

#[tokio::test]
async fn model_uses_max_completion_tokens_reads_the_vendor_rules() {
    let mut reasoning = FakePredicates { reasoning: true };
    assert!(uses_max_completion_tokens("google.gemini-2.5-flash", &mut reasoning).await);
    let mut plain = FakePredicates { reasoning: false };
    assert!(!uses_max_completion_tokens("google.gemini-2.5-flash", &mut plain).await);
    assert!(uses_max_completion_tokens("openai.gpt-5", &mut plain).await);
    assert!(!uses_max_completion_tokens("OCI/openai.gpt-oss-120b", &mut plain).await);
    assert!(!uses_max_completion_tokens("", &mut plain).await);
}

fn generic_body() -> serde_json::Value {
    json!({
        "modelId": "meta.llama",
        "modelVersion": "1",
        "chatResponse": {
            "apiFormat": "GENERIC",
            "timeCreated": "2025-01-01T00:00:00.000Z",
            "choices": [{
                "index": 0,
                "finishReason": "TOOL_CALL",
                "message": {
                    "role": "ASSISTANT",
                    "content": [{"type": "TEXT", "text": "hi "}, {"type": "TEXT", "text": "there"}],
                    "toolCalls": [{"id": "call_1", "type": "FUNCTION", "name": "lookup", "arguments": "{}"}],
                },
            }],
            "usage": {"promptTokens": 3, "completionTokens": 5, "totalTokens": 8},
        },
    })
}

#[test]
fn response_decodes_a_generic_body() {
    let body = serde_json::to_vec(&generic_body()).unwrap();
    let reply = transform_response(
        "meta.llama-3.3-70b-instruct",
        200,
        &body,
        &IndexMap::new(),
        &FixedClock(0),
    )
    .unwrap();
    assert_eq!(reply.response.model, "meta.llama");
    assert_eq!(reply.response.created, 1_735_689_600);
    assert_eq!(
        reply.response.choices[0].message.content.as_deref(),
        Some("hi there")
    );
    assert_eq!(
        reply.response.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
    let tool_calls = reply.response.choices[0]
        .message
        .tool_calls
        .as_ref()
        .unwrap();
    assert_eq!(tool_calls[0].id, "call_1");
    assert_eq!(reply.response.usage.total_tokens, 8);
}

#[test]
fn response_decodes_a_cohere_body() {
    let body = serde_json::to_vec(&json!({
        "modelId": "cohere.command-r",
        "modelVersion": "1.0",
        "chatResponse": {
            "text": "hello",
            "apiFormat": "COHERE",
            "finishReason": "COMPLETE",
            "usage": {"promptTokens": 2, "completionTokens": 3, "totalTokens": 5},
        },
    }))
    .unwrap();
    let reply = transform_response(
        "cohere.command-r",
        200,
        &body,
        &IndexMap::new(),
        &FixedClock(1_700_000_000),
    )
    .unwrap();
    assert_eq!(reply.response.model, "cohere.command-r");
    assert_eq!(reply.response.created, 1_700_000_000);
    assert_eq!(
        reply.response.choices[0].message.content.as_deref(),
        Some("hello")
    );
    assert_eq!(
        reply.response.choices[0].finish_reason.as_deref(),
        Some("stop")
    );
}

#[test]
fn response_maps_the_error_key_and_non_objects() {
    let error_key = transform_response(
        "meta.llama-3.3-70b-instruct",
        502,
        br#"{"error": "boom"}"#,
        &IndexMap::new(),
        &FixedClock(0),
    )
    .unwrap_err();
    assert_eq!(error_key.status, 502);
    assert_eq!(error_key.message, "boom");
    let non_object = transform_response(
        "meta.llama-3.3-70b-instruct",
        201,
        b"[]",
        &IndexMap::new(),
        &FixedClock(0),
    )
    .unwrap_err();
    assert_eq!(non_object.status, 201);
    assert_eq!(non_object.message, "Invalid response format from OCI");
    let bad_json = decode_chat_body(200, b"{oops").unwrap_err();
    assert_eq!(bad_json.status, 200);
    assert!(
        bad_json
            .message
            .starts_with("Invalid response format from OCI:")
    );
    let mut at_limit = vec![b' '; crate::oci_chat::response::MAX_CHAT_BODY_BYTES];
    at_limit[..2].copy_from_slice(b"{}");
    assert!(decode_chat_body(200, at_limit.as_slice()).is_ok());
    let oversized = vec![b' '; crate::oci_chat::response::MAX_CHAT_BODY_BYTES + 1];
    let too_large = decode_chat_body(200, oversized.as_slice()).unwrap_err();
    assert_eq!(too_large.status, 200);
    assert!(too_large.message.contains("larger than"));
}

#[test]
fn response_keeps_the_response_headers() {
    let mut headers = IndexMap::new();
    headers.insert(String::from("x-request-id"), String::from("abc"));
    let body = serde_json::to_vec(&generic_body()).unwrap();
    let reply = transform_response(
        "meta.llama-3.3-70b-instruct",
        200,
        &body,
        &headers,
        &FixedClock(0),
    )
    .unwrap();
    let value = serde_json::to_value(&reply.additional_headers).unwrap();
    assert_eq!(value["llm_provider-x-request-id"], json!("abc"));
}

#[test]
fn sse_decoder_waits_for_a_full_line() {
    let mut decoder = SseDecoder::default();
    let first = decoder
        .push("data: {\"message\":{\"content\":[{\"type\":\"TEXT\",\"text\":\"he\"}]}}")
        .unwrap();
    assert!(first.is_empty());
    let rest = decoder.push("\ndata: [DONE]\n").unwrap();
    assert_eq!(rest.len(), 2);
    let StreamItem::Chunk(chunk) = &rest[0] else {
        panic!("the first line carries a chunk");
    };
    assert_eq!(chunk.choice.delta.content.as_deref(), Some("he"));
    assert!(matches!(rest[1], StreamItem::Done));
}

#[test]
fn sse_decoder_flushes_a_trailing_line() {
    let mut decoder = SseDecoder::default();
    assert!(
        decoder
            .push("data: {\"message\":{\"content\":[{\"type\":\"TEXT\",\"text\":\"end\"}]}}")
            .unwrap()
            .is_empty()
    );
    let flushed = decoder.finish().unwrap();
    assert_eq!(flushed.len(), 1);
    assert!(decoder.finish().unwrap().is_empty());
}

#[test]
fn sse_decoder_reports_tool_calls_and_finish_reason() {
    let mut decoder = SseDecoder::default();
    let items = decoder
        .push("data: {\"index\":0,\"message\":{\"toolCalls\":[{\"name\":\"lookup\",\"arguments\":\"{}\"}]},\"finishReason\":\"TOOL_CALL\"}\n")
        .unwrap();
    let StreamItem::Chunk(chunk) = &items[0] else {
        panic!("the line carries a chunk");
    };
    assert!(chunk.tool_call);
    assert_eq!(chunk.finish_reason.as_deref(), Some("tool_calls"));
    let items = decoder
        .push("data: {\"index\":0,\"message\":{\"content\":[]}}\n")
        .unwrap();
    let StreamItem::Chunk(chunk) = &items[0] else {
        panic!("the line carries a chunk");
    };
    assert!(!chunk.tool_call);
    assert!(chunk.finish_reason.is_none());
}

#[test]
fn sse_decoder_suppresses_cohere_restatements() {
    let mut decoder = SseDecoder::default();
    let items = decoder
        .push("data: {\"apiFormat\":\"COHERE\",\"index\":0,\"text\":\"hi\"}\n")
        .unwrap();
    let StreamItem::Chunk(chunk) = &items[0] else {
        panic!("the line carries a chunk");
    };
    assert_eq!(chunk.choice.delta.content.as_deref(), Some("hi"));
    let items = decoder
        .push("data: {\"apiFormat\":\"COHERE\",\"index\":0,\"text\":\"hi\",\"chatHistory\":[]}\n")
        .unwrap();
    let StreamItem::Chunk(chunk) = &items[0] else {
        panic!("the line carries a chunk");
    };
    assert!(chunk.choice.delta.content.is_none());
}

#[test]
fn sse_decoder_marks_cohere_state_only_after_real_content() {
    let mut decoder = SseDecoder::default();
    decoder
        .push("data: {\"apiFormat\":\"COHERE\",\"index\":0,\"text\":\"\"}\n")
        .unwrap();
    let items = decoder
        .push("data: {\"apiFormat\":\"COHERE\",\"index\":0,\"text\":\"x\",\"chatHistory\":[]}\n")
        .unwrap();
    let StreamItem::Chunk(chunk) = &items[0] else {
        panic!("the line carries a chunk");
    };
    assert_eq!(chunk.choice.delta.content.as_deref(), Some("x"));
    let mut decoder = SseDecoder::default();
    decoder
        .push("data: {\"apiFormat\":\"COHERE\",\"index\":0,\"text\":\"a\"}\n")
        .unwrap();
    let call = "\"toolCalls\":[{\"name\":\"lookup\",\"parameters\":{}}]";
    let first = decoder
        .push(&format!(
            "data: {{\"apiFormat\":\"COHERE\",\"index\":0,\"chatHistory\":[],{call}}}\n"
        ))
        .unwrap();
    let StreamItem::Chunk(chunk) = &first[0] else {
        panic!("the line carries a chunk");
    };
    assert!(chunk.tool_call);
    let again = decoder
        .push(&format!(
            "data: {{\"apiFormat\":\"COHERE\",\"index\":0,\"chatHistory\":[],{call}}}\n"
        ))
        .unwrap();
    let StreamItem::Chunk(chunk) = &again[0] else {
        panic!("the line carries a chunk");
    };
    assert!(!chunk.tool_call);
}

#[test]
fn sse_decoder_handles_other_lines_and_bad_json() {
    let mut decoder = SseDecoder::default();
    let items = decoder.push("event: ping\n: comment\n\n").unwrap();
    assert!(items.is_empty());
    let error = decoder.push("data: {oops}\n").unwrap_err();
    assert_eq!(error.status, 500);
    assert!(error.message.starts_with("Chunk cannot be parsed as JSON"));
}

#[test]
fn stream_status_maps_non_200() {
    assert!(check_stream_status(200, "").is_ok());
    let error = check_stream_status(503, "unavailable").unwrap_err();
    assert_eq!(error.status, 503);
    assert_eq!(error.message, "unavailable");
}

#[test]
fn streaming_timeout_is_300_seconds() {
    assert_eq!(STREAMING_TIMEOUT, Duration::from_secs(300));
}

#[test]
fn replays_oci_chat_bytes_corpus() {
    let seen = fs::read_dir("fuzz/corpus/oci_chat_bytes")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _accepted = decode_chat_body(200, &bytes);
            let _rejected = decode_chat_body(500, &bytes);
            if let Ok(text) = std::str::from_utf8(&bytes) {
                let mut decoder = SseDecoder::default();
                let _ = decoder.push(text);
                let _ = decoder.finish();
            }
        })
        .count();
    assert!(seen > 0_usize);
}
