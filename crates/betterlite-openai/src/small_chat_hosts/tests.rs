#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json as HiddenJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use super::maritalk::MaritalkConfig;
use super::meta_llama::LlamaAPIConfig;
use super::moonshot::{self, MoonshotError, TransformInput, complete_url, map_openai_params};
use super::morph::MorphChatConfig;
use super::nadir::{self, NadirReply, ReplyRequest as NadirRequest, nadir_reported_cost};
use super::nebius::NebiusConfig;
use super::novita::{MISSING_KEY_MESSAGE, NovitaConfig, NovitaKeyError};
use super::nscale::NscaleConfig;
use super::v0::V0ChatConfig;
use super::wandb::WandbConfig;
use super::zai::ZaiChatConfig;
use super::{CompletionTokenRename, json_is_truthy, map_supported};
use crate::chat::{ChatMessage, ContentPart, FunctionCall, MessageContent, Role, Tool, ToolCall};
use crate::chat_choices::reply::{ResponseReply, TransformResponseError};
use crate::chat_content::{ChatContentError, MissingFileError, Mode, RefuseDownloads};
use crate::compatible_wire::{LogInput, ReplyOptions};
use crate::edenai_chat::ReasoningLookup;
use crate::edenai_common::RESPONSE_COST_HEADER;
use crate::openai_base_config::forwarding::ForwardingFailure;
use crate::openai_base_config::{CatalogCapabilities, Headers, Object as WireObject};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const MODEL: &str = "a-model";

struct Reasons(bool);

impl ReasoningLookup for Reasons {
    type Error = Infallible;

    fn supports_reasoning(&self, _model: &str) -> Result<bool, Infallible> {
        Ok(self.0)
    }
}

#[derive(Debug, Error)]
#[error("the capability lookup failed")]
struct LookupFailed;

struct Failing;

impl ReasoningLookup for Failing {
    type Error = LookupFailed;

    fn supports_reasoning(&self, _model: &str) -> Result<bool, LookupFailed> {
        Err(LookupFailed)
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn number(value: f64) -> Json {
    Json::Number(serde_json::Number::from_f64(value).expect("a finite fixture number"))
}

fn integer(value: u64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(pairs: Vec<(&str, Json)>) -> Object {
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn mapped(pairs: Vec<(&str, Json)>) -> Object {
    let mut optional = Object::new();
    map_openai_params(
        &Reasons(false),
        MODEL,
        &object(pairs),
        &mut optional,
        &catalog(),
    );
    optional
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

fn tool_message(passthrough: IndexMap<String, Value>) -> ChatMessage {
    ChatMessage {
        role: Role::Assistant,
        content: None,
        name: None,
        tool_calls: Some(vec![ToolCall {
            id: String::from("call-1"),
            kind: String::from("function"),
            function: FunctionCall {
                name: String::from("lookup"),
                arguments: String::from("{}"),
            },
        }]),
        tool_call_id: None,
        passthrough,
    }
}

fn parts_message(parts: Vec<ContentPart>) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        content: Some(MessageContent::Parts(parts)),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

fn part(kind: &str, text: &str) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields: IndexMap::from([(String::from("text"), Value::String(text.to_owned()))]),
    }
}

fn moonshot_body(
    reasoning: bool,
    messages: &[ChatMessage],
    optional: &mut Object,
    mode: Mode,
) -> Object {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let downloads = RefuseDownloads;
    let input = TransformInput {
        model: MODEL,
        messages,
        optional_params: optional,
        environment: &environment,
        downloads: &downloads,
    };
    moonshot::transform_request(&Reasons(reasoning), input, mode).expect("the body transforms")
}

fn body_messages(body: &Object) -> Vec<Json> {
    let Some(Json::List(messages)) = body.get("messages") else {
        panic!("the body carries messages: {body:?}");
    };
    messages.clone()
}

fn only_message(body: &Object) -> Json {
    let messages = body_messages(body);
    assert_eq!(messages.len(), 1);
    messages.into_iter().next().expect("one message")
}

#[test]
fn maritalk_lists_exactly_the_twelve_params_in_order() {
    assert_eq!(
        MaritalkConfig::supported_params(),
        vec![
            "frequency_penalty",
            "presence_penalty",
            "top_p",
            "top_k",
            "temperature",
            "max_tokens",
            "n",
            "stop",
            "stream",
            "stream_options",
            "tools",
            "tool_choice",
        ]
    );
}

#[test]
fn maritalk_error_carries_message_status_and_headers() {
    let headers = Headers::from([(String::from("x-request-id"), String::from("req-1"))]);
    let error = MaritalkConfig::get_error_class("bad request", 422, Some(&headers));
    assert_eq!(error.error.status_code, 422);
    assert_eq!(error.error.message, "bad request");
    assert_eq!(error.to_string(), "bad request");
    assert_eq!(
        error.error.headers,
        Some(vec![(String::from("x-request-id"), String::from("req-1"))])
    );
    let bare = MaritalkConfig::get_error_class("gone", 410, None);
    assert_eq!(bare.error.headers, None);
}

#[test]
fn llama_supported_params_are_the_openai_list() {
    assert_eq!(
        LlamaAPIConfig::supported_params(MODEL, &catalog()),
        super::openai_parent().supported_params(MODEL, &catalog())
    );
}

#[test]
fn llama_keeps_a_json_schema_response_format() {
    let keep = serde_json::json!({"type": "json_schema", "json_schema": {"name": "x"}});
    let mut optional = WireObject::new();
    LlamaAPIConfig::map_openai_params(
        &WireObject::from_iter([(String::from("response_format"), keep.clone())]),
        &mut optional,
        MODEL,
        &catalog(),
    );
    assert_eq!(optional.get("response_format"), Some(&keep));
}

#[rstest]
#[case::text(serde_json::json!({"type": "text"}))]
#[case::no_type(serde_json::json!({"json_schema": {"name": "x"}}))]
fn llama_drops_a_response_format_that_is_not_json_schema(#[case] dropped: Value) {
    let mut optional = WireObject::new();
    LlamaAPIConfig::map_openai_params(
        &WireObject::from_iter([(String::from("response_format"), dropped)]),
        &mut optional,
        MODEL,
        &catalog(),
    );
    assert!(!optional.contains_key("response_format"));
}

#[test]
fn llama_maps_the_other_params_through_the_parent() {
    let mut optional = WireObject::new();
    LlamaAPIConfig::map_openai_params(
        &WireObject::from_iter([
            (String::from("temperature"), Value::from(0.5_f64)),
            (String::from("tools"), serde_json::json!([])),
            (String::from("unknown_param"), Value::Bool(true)),
        ]),
        &mut optional,
        MODEL,
        &catalog(),
    );
    assert_eq!(optional.get("temperature"), Some(&Value::from(0.5_f64)));
    assert_eq!(optional.get("tools"), Some(&serde_json::json!([])));
    assert!(!optional.contains_key("unknown_param"));
}

#[test]
fn morph_and_v0_list_their_small_param_sets() {
    assert_eq!(
        MorphChatConfig::supported_params(MODEL),
        vec!["messages", "model", "stream"]
    );
    assert_eq!(super::morph::PROVIDER, "morph");
    assert_eq!(
        V0ChatConfig::supported_params(MODEL),
        vec!["messages", "model", "stream", "tools", "tool_choice"]
    );
    assert_eq!(super::v0::PROVIDER, "v0");
}

#[test]
fn nebius_keeps_the_openai_list() {
    assert_eq!(
        NebiusConfig::supported_params(MODEL, &catalog()),
        super::openai_parent().supported_params(MODEL, &catalog())
    );
}

#[test]
fn nebius_maps_max_completion_tokens_and_skips_the_rest() {
    let mut optional = Object::new();
    NebiusConfig::map_openai_params(
        MODEL,
        &object(vec![
            ("max_completion_tokens", number(64.0)),
            ("temperature", number(0.5)),
            ("unknown_param", Json::Bool(true)),
        ]),
        &mut optional,
        &catalog(),
    );
    assert_eq!(optional.get("max_tokens"), Some(&number(64.0)));
    assert_eq!(optional.get("temperature"), Some(&number(0.5)));
    assert!(!optional.contains_key("max_completion_tokens"));
    assert!(!optional.contains_key("unknown_param"));
}

#[test]
fn nebius_takes_the_later_of_max_tokens_and_max_completion_tokens() {
    let mut first = Object::new();
    NebiusConfig::map_openai_params(
        MODEL,
        &object(vec![
            ("max_tokens", number(10.0)),
            ("max_completion_tokens", number(20.0)),
        ]),
        &mut first,
        &catalog(),
    );
    assert_eq!(first.get("max_tokens"), Some(&number(20.0)));
    let mut second = Object::new();
    NebiusConfig::map_openai_params(
        MODEL,
        &object(vec![
            ("max_completion_tokens", number(20.0)),
            ("max_tokens", number(10.0)),
        ]),
        &mut second,
        &catalog(),
    );
    assert_eq!(second.get("max_tokens"), Some(&number(10.0)));
}

#[test]
fn wandb_adds_reasoning_effort_only_for_a_reasoning_model() {
    let parent = super::openai_parent().supported_params(MODEL, &catalog());
    let mut reasoning = WandbConfig::supported_params(&Reasons(true), MODEL, &catalog());
    assert_eq!(reasoning.len(), parent.len() + 1);
    assert_eq!(reasoning.pop(), Some("reasoning_effort"));
    assert_eq!(
        WandbConfig::supported_params(&Reasons(false), MODEL, &catalog()),
        parent
    );
}

#[test]
fn wandb_maps_like_nebius_and_copies_reasoning_effort() {
    let mut optional = Object::new();
    WandbConfig::map_openai_params(
        &Reasons(true),
        MODEL,
        &object(vec![
            ("max_completion_tokens", number(64.0)),
            ("reasoning_effort", text("high")),
            ("n", number(2.0)),
        ]),
        &mut optional,
        &catalog(),
    );
    assert_eq!(optional.get("max_tokens"), Some(&number(64.0)));
    assert_eq!(optional.get("reasoning_effort"), Some(&text("high")));
    assert_eq!(optional.get("n"), Some(&number(2.0)));
}

#[test]
fn novita_sets_the_three_headers_on_a_copy() {
    let given = Headers::from([(String::from("X-Caller"), String::from("kept"))]);
    let key = SecretString::from(String::from("sk-novita"));
    let headers = NovitaConfig::validate_environment(Some(&key), &given).expect("a key is present");
    assert_eq!(
        headers.get("Authorization"),
        Some(&String::from("Bearer sk-novita"))
    );
    assert_eq!(
        headers.get("Content-Type"),
        Some(&String::from("application/json"))
    );
    assert_eq!(
        headers.get("X-Novita-Source"),
        Some(&String::from("litellm"))
    );
    assert_eq!(headers.get("X-Caller"), Some(&String::from("kept")));
    assert_eq!(given.len(), 1);
}

#[test]
fn novita_accepts_an_empty_key() {
    let empty = SecretString::from(String::new());
    let headers = NovitaConfig::validate_environment(Some(&empty), &Headers::new())
        .expect("an empty key is present");
    assert_eq!(headers.get("Authorization"), Some(&String::from("Bearer ")));
}

#[test]
fn novita_fails_without_a_key_and_keeps_the_litellm_message() {
    let error =
        NovitaConfig::validate_environment(None, &Headers::new()).expect_err("no key is an error");
    assert_eq!(error, NovitaKeyError);
    assert_eq!(error.to_string(), MISSING_KEY_MESSAGE);
}

#[test]
fn nscale_lists_exactly_the_twelve_params_in_order() {
    assert_eq!(
        NscaleConfig::supported_params(MODEL),
        vec![
            "max_tokens",
            "n",
            "temperature",
            "top_p",
            "stream",
            "logprobs",
            "top_logprobs",
            "frequency_penalty",
            "presence_penalty",
            "response_format",
            "stop",
            "logit_bias",
        ]
    );
    assert_eq!(super::nscale::PROVIDER, "nscale");
}

#[test]
fn zai_lists_eight_params_then_thinking_for_a_reasoning_model() {
    let base = vec![
        "max_tokens",
        "stream",
        "stream_options",
        "temperature",
        "top_p",
        "stop",
        "tools",
        "tool_choice",
    ];
    assert_eq!(
        ZaiChatConfig::supported_params(&Reasons(false), MODEL),
        base
    );
    let mut reasoning = ZaiChatConfig::supported_params(&Reasons(true), MODEL);
    assert_eq!(reasoning.pop(), Some("thinking"));
    assert_eq!(reasoning, base);
    assert_eq!(super::zai::PROVIDER, "zai");
}

#[test]
fn zai_lookup_failure_means_no_thinking_and_no_error() {
    assert_eq!(
        ZaiChatConfig::supported_params(&Failing, MODEL),
        ZaiChatConfig::supported_params(&Reasons(false), MODEL)
    );
}

#[test]
fn zai_keeps_cache_control_on_messages_and_tools() {
    let mut cached = message(Role::User, "hi");
    cached.passthrough.insert(
        String::from("cache_control"),
        serde_json::json!({"type": "ephemeral"}),
    );
    let tool = Tool {
        kind: String::from("function"),
        function: None,
        passthrough: IndexMap::from([(
            String::from("cache_control"),
            serde_json::json!({"type": "ephemeral"}),
        )]),
    };
    let (messages, tools) = ZaiChatConfig::remove_cache_control_flag_from_messages_and_tools(
        MODEL,
        &[cached.clone()],
        Some(&[tool.clone()]),
    );
    assert_eq!(messages, vec![cached]);
    assert_eq!(tools, Some(vec![tool]));
}

#[test]
fn moonshot_complete_url_builds_the_default_and_appends_the_path() {
    assert_eq!(
        complete_url(None),
        "https://api.moonshot.ai/v1/chat/completions"
    );
    assert_eq!(
        complete_url(Some("")),
        "https://api.moonshot.ai/v1/chat/completions"
    );
    assert_eq!(
        complete_url(Some("https://api.moonshot.ai/v1")),
        "https://api.moonshot.ai/v1/chat/completions"
    );
    assert_eq!(
        complete_url(Some("https://proxy.example/v1")),
        "https://proxy.example/v1/chat/completions"
    );
    assert_eq!(
        complete_url(Some("https://proxy.example/chat/completions")),
        "https://proxy.example/chat/completions"
    );
    // the base is never trimmed
    assert_eq!(
        complete_url(Some("https://proxy.example/v1/")),
        "https://proxy.example/v1//chat/completions"
    );
}

#[test]
fn moonshot_supported_params_drop_functions_and_gate_tool_calls() {
    let parent = super::openai_parent().supported_params(MODEL, &catalog());
    let plain = moonshot::supported_params(&Reasons(false), MODEL, &catalog());
    assert_eq!(plain.len(), parent.len() - 1);
    assert!(!plain.contains(&"functions"));
    assert!(plain.contains(&"tools"));
    let thinking = moonshot::supported_params(
        &Reasons(false),
        "moonshot/kimi-thinking-preview",
        &catalog(),
    );
    assert_eq!(thinking.len(), parent.len() - 3);
    assert!(!thinking.contains(&"tools"));
    assert!(!thinking.contains(&"tool_choice"));
    let mut reasoning = moonshot::supported_params(&Reasons(true), MODEL, &catalog());
    assert_eq!(reasoning.pop(), Some("reasoning_effort"));
}

#[test]
fn moonshot_maps_in_input_order_and_unwraps_reasoning_effort() {
    assert_eq!(
        mapped(vec![
            ("max_completion_tokens", number(64.0)),
            ("temperature", number(0.5)),
            ("functions", Json::List(Vec::new())),
            ("unknown_param", Json::Bool(true)),
        ]),
        object(vec![
            ("max_tokens", number(64.0)),
            ("temperature", number(0.5)),
        ])
    );
    let mut unwrapped = Object::new();
    map_openai_params(
        &Reasons(true),
        MODEL,
        &object(vec![
            (
                "reasoning_effort",
                Json::Object(object(vec![("effort", text("low"))])),
            ),
            ("reasoning_effort_object", Json::Null),
        ]),
        &mut unwrapped,
        &catalog(),
    );
    assert_eq!(unwrapped.get("reasoning_effort"), Some(&text("low")));
    let mut bare = Object::new();
    map_openai_params(
        &Reasons(true),
        MODEL,
        &object(vec![("reasoning_effort", text("high"))]),
        &mut bare,
        &catalog(),
    );
    assert_eq!(bare.get("reasoning_effort"), Some(&text("high")));
    let mut skipped = Object::new();
    map_openai_params(
        &Reasons(true),
        MODEL,
        &object(vec![("reasoning_effort", number(2.0))]),
        &mut skipped,
        &catalog(),
    );
    assert!(!skipped.contains_key("reasoning_effort"));
}

#[test]
fn moonshot_removes_temperature_for_a_reasoning_model() {
    let mut optional = Object::from_iter([(String::from("temperature"), number(0.5))]);
    map_openai_params(
        &Reasons(true),
        MODEL,
        &object(Vec::new()),
        &mut optional,
        &catalog(),
    );
    assert!(!optional.contains_key("temperature"));
}

#[test]
fn moonshot_caps_temperature_and_raises_the_low_pair() {
    assert_eq!(
        mapped(vec![("temperature", number(1.5))]).get("temperature"),
        Some(&integer(1))
    );
    assert_eq!(
        mapped(vec![("temperature", number(1.0))]).get("temperature"),
        Some(&number(1.0))
    );
    assert_eq!(
        mapped(vec![("temperature", number(0.2)), ("n", number(2.0))]).get("temperature"),
        Some(&number(0.3))
    );
    assert_eq!(
        mapped(vec![("temperature", number(0.3)), ("n", number(2.0))]).get("temperature"),
        Some(&number(0.3))
    );
    assert_eq!(
        mapped(vec![("temperature", number(0.2)), ("n", number(1.0))]).get("temperature"),
        Some(&number(0.2))
    );
    assert_eq!(
        mapped(vec![("temperature", number(0.2))]).get("temperature"),
        Some(&number(0.2))
    );
    assert_eq!(
        mapped(vec![("temperature", number(0.2)), ("n", number(2.0))]).get("n"),
        Some(&number(2.0))
    );
}

#[test]
fn moonshot_keeps_a_temperature_exactly_at_the_floor() {
    let at_floor: Json = serde_json::from_str("0.30").expect("a number");
    let optional = mapped(vec![("temperature", at_floor.clone()), ("n", number(2.0))]);
    assert_eq!(optional.get("temperature"), Some(&at_floor));
}

#[test]
fn moonshot_rewrites_tool_choice_required_and_drops_the_key() {
    let messages = vec![message(Role::User, "hi")];
    let mut optional = Object::from_iter([(String::from("tool_choice"), text("required"))]);
    let body = moonshot_body(false, &messages, &mut optional, Mode::Sync);
    assert!(!optional.contains_key("tool_choice"));
    let sent = body_messages(&body);
    assert_eq!(sent.len(), 2);
    let Some(Json::Object(last)) = sent.last() else {
        panic!("the appended message is an object");
    };
    assert_eq!(last.get("role"), Some(&text("user")));
    assert_eq!(
        last.get("content"),
        Some(&text("Please select a tool to handle the current issue."))
    );
}

#[test]
fn moonshot_leaves_another_tool_choice_alone() {
    let messages = vec![message(Role::User, "hi")];
    let mut optional = Object::from_iter([(String::from("tool_choice"), text("auto"))]);
    let body = moonshot_body(false, &messages, &mut optional, Mode::Sync);
    assert_eq!(optional.get("tool_choice"), Some(&text("auto")));
    assert_eq!(body_messages(&body).len(), 1);
}

#[test]
fn moonshot_flattens_text_only_lists() {
    let messages = vec![
        parts_message(vec![part("text", "hello "), part("text", "world")]),
        message(Role::User, "plain"),
    ];
    let mut optional = Object::new();
    let body = moonshot_body(false, &messages, &mut optional, Mode::Sync);
    let sent = body_messages(&body);
    assert_eq!(
        sent[0],
        Json::Object(object(vec![
            ("role", text("user")),
            ("content", text("hello world"))
        ]))
    );
    assert_eq!(
        sent[1],
        Json::Object(object(vec![
            ("role", text("user")),
            ("content", text("plain"))
        ]))
    );
}

#[test]
fn moonshot_keeps_every_list_when_any_part_is_not_text() {
    let messages = vec![
        parts_message(vec![part("text", "hello")]),
        parts_message(vec![part("image_url", "https://example.test/i.png")]),
    ];
    let mut optional = Object::new();
    let body = moonshot_body(false, &messages, &mut optional, Mode::Sync);
    let sent = body_messages(&body);
    let Some(Json::Object(first)) = sent.first() else {
        panic!("a message object");
    };
    assert!(matches!(first.get("content"), Some(Json::List(_))));
}

#[test]
fn moonshot_fills_reasoning_content_from_provider_specific_fields() {
    let passthrough = IndexMap::from([(
        String::from("provider_specific_fields"),
        serde_json::json!({"reasoning_content": "stored", "kept": true}),
    )]);
    let messages = vec![tool_message(passthrough)];
    let mut optional = Object::new();
    let body = moonshot_body(true, &messages, &mut optional, Mode::Sync);
    let sent = only_message(&body);
    let Json::Object(fields) = sent else {
        panic!("a message object");
    };
    assert_eq!(fields.get("reasoning_content"), Some(&text("stored")));
    let Some(Json::Object(provider)) = fields.get("provider_specific_fields") else {
        panic!("the provider fields stay an object");
    };
    assert!(!provider.contains_key("reasoning_content"));
    assert_eq!(provider.get("kept"), Some(&Json::Bool(true)));
}

#[test]
fn moonshot_injects_a_space_when_no_reasoning_content_is_stored() {
    let messages = vec![tool_message(IndexMap::new())];
    let mut optional = Object::new();
    let body = moonshot_body(true, &messages, &mut optional, Mode::Sync);
    let Json::Object(fields) = only_message(&body) else {
        panic!("a message object");
    };
    assert_eq!(fields.get("reasoning_content"), Some(&text(" ")));
}

#[test]
fn moonshot_leaves_messages_that_already_carry_reasoning_content() {
    let passthrough = IndexMap::from([(
        String::from("reasoning_content"),
        Value::String(String::from("own")),
    )]);
    let messages = vec![tool_message(passthrough)];
    let mut optional = Object::new();
    let body = moonshot_body(true, &messages, &mut optional, Mode::Sync);
    let Json::Object(fields) = only_message(&body) else {
        panic!("a message object");
    };
    assert_eq!(fields.get("reasoning_content"), Some(&text("own")));
}

#[test]
fn moonshot_fills_nothing_for_a_model_that_does_not_reason() {
    let messages = vec![tool_message(IndexMap::new())];
    let mut optional = Object::new();
    let body = moonshot_body(false, &messages, &mut optional, Mode::Sync);
    let Json::Object(fields) = only_message(&body) else {
        panic!("a message object");
    };
    assert!(!fields.contains_key("reasoning_content"));
}

#[test]
fn moonshot_leaves_an_assistant_message_with_empty_tool_calls_alone() {
    let mut bare = message(Role::Assistant, "done");
    bare.tool_calls = Some(Vec::new());
    let body = moonshot_body(true, &[bare], &mut Object::new(), Mode::Sync);
    let sent = serde_json::to_string(&only_message(&body)).expect("the message JSON");
    assert!(!sent.contains("reasoning_content"));
}

#[test]
fn moonshot_transforms_the_async_call_like_the_sync_one() {
    let messages = vec![message(Role::User, "hi")];
    let mut sync_optional = Object::new();
    let sync_body = moonshot_body(false, &messages, &mut sync_optional, Mode::Sync);
    let mut async_optional = Object::new();
    let async_body = moonshot_body(false, &messages, &mut async_optional, Mode::AsyncSubclass);
    assert_eq!(sync_body, async_body);
}

#[test]
fn moonshot_error_status_follows_the_content_error() {
    let file = MissingFileError {
        provider: "openai",
        model: None,
    };
    let missing = MoonshotError::Content(ChatContentError::MissingFile(file));
    assert_eq!(missing.status_code(), 400);
    let source = serde_json::from_str::<Value>("nope").expect_err("not JSON");
    let shape = MoonshotError::Content(ChatContentError::MessageShape { source });
    assert_eq!(shape.status_code(), 500);
}

#[test]
fn json_truthiness_matches_the_python_read() {
    assert!(json_is_truthy(&Value::Bool(true)));
    assert!(!json_is_truthy(&Value::Bool(false)));
    assert!(!json_is_truthy(&Value::Null));
    assert!(!json_is_truthy(&Value::String(String::new())));
    assert!(json_is_truthy(&serde_json::json!([1])));
    assert!(!json_is_truthy(&serde_json::json!({})));
    assert!(json_is_truthy(&serde_json::json!(1)));
    assert!(!json_is_truthy(&serde_json::json!(0.0)));
}

#[test]
fn map_supported_copies_only_a_listed_param() {
    let mut optional = object(vec![("kept", text("old"))]);
    let requested = object(vec![("kept", text("new")), ("unknown", Json::Bool(true))]);
    map_supported(
        &requested,
        &mut optional,
        &["kept"],
        CompletionTokenRename::Keep,
    );
    assert_eq!(optional, object(vec![("kept", text("new"))]));
}

#[test]
fn nadir_lists_exactly_the_nine_params_in_order() {
    assert_eq!(
        nadir::NadirConfig::supported_params(),
        vec![
            "extra_headers",
            "frequency_penalty",
            "max_retries",
            "max_tokens",
            "presence_penalty",
            "response_format",
            "stream",
            "temperature",
            "top_p",
        ]
    );
}

#[rstest]
#[case(br#"{"nadir_metadata":{"cost":{"total_cost_usd":1.5}}}"#, Some(1.5))]
#[case(br#"{"nadir_metadata":{"cost":{"total_cost_usd":2}}}"#, Some(2.0))]
#[case(br#"{"nadir_metadata":{"cost":{"total_cost_usd":0}}}"#, Some(0.0))]
#[case(br#"{"nadir_metadata":{"cost":{"total_cost_usd":true}}}"#, None)]
#[case(br#"{"nadir_metadata":{"cost":{"total_cost_usd":false}}}"#, None)]
#[case(br#"{"nadir_metadata":{"cost":{"total_cost_usd":-1}}}"#, None)]
#[case(br#"{"nadir_metadata":{"cost":{"total_cost_usd":"1.5"}}}"#, None)]
#[case(br#"{"nadir_metadata":{"cost":{"total_cost_usd":null}}}"#, None)]
#[case(br#"{"nadir_metadata":{"cost":{}}}"#, None)]
#[case(br#"{"nadir_metadata":{}}"#, None)]
#[case(b"{}", None)]
#[case(br#"{"nadir_metadata":[]}"#, None)]
#[case(br#"{"nadir_metadata":{"cost":"x"}}"#, None)]
#[case(b"not json", None)]
#[case(b"", None)]
fn nadir_reported_cost_reads_only_a_finite_non_negative_number(
    #[case] body: &[u8],
    #[case] expected: Option<f64>,
) {
    assert_eq!(nadir_reported_cost(body), expected);
}

#[test]
fn nadir_reports_an_integer_cost_as_a_float() {
    let cost = nadir_reported_cost(br#"{"nadir_metadata":{"cost":{"total_cost_usd":7}}}"#);
    assert_eq!(cost, Some(7.0_f64));
    assert_eq!(cost.map(|value| value.to_string()), Some(String::from("7")));
}

#[test]
fn nadir_treats_a_cost_beyond_f64_as_no_cost() {
    let zeros = "0".repeat(400);
    let body = format!("{{\"nadir_metadata\":{{\"cost\":{{\"total_cost_usd\":1{zeros}}}}}}}");
    assert_eq!(nadir_reported_cost(body.as_bytes()), None);
}

fn identity_projection(
    decoded: Value,
    _response: &Headers,
    _hidden: &Headers,
) -> Result<Value, ForwardingFailure> {
    Ok(decoded)
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn fixture(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:05:00+00:00").unwrap(),
    };
    (state, env)
}

fn log_input() -> LogInput {
    LogInput {
        messages: HiddenJson::List(vec![HiddenJson::Object(IndexMap::from([(
            String::from("role"),
            HiddenJson::Text(String::from("user")),
        )]))]),
        data: RawBody::Object(IndexMap::from([(
            String::from("model"),
            HiddenJson::Text(String::from("m")),
        )])),
    }
}

fn reply_options() -> ReplyOptions<'static> {
    ReplyOptions {
        json_mode: false,
        provider: Some("nadir"),
        base_model: None,
        identifier: Uuid::nil,
        clock: || 7,
    }
}

fn nadir_reply(body: &[u8]) -> Result<NadirReply, TransformResponseError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = fixture(&redactor);
    let headers = Headers::from([(String::from("x-request-id"), String::from("req-1"))]);
    nadir::transform_response(
        NadirRequest {
            reply: ResponseReply {
                status: 200,
                headers: &headers,
                body,
            },
            state: &mut state,
            env: &env,
            input: log_input(),
            options: reply_options(),
        },
        identity_projection,
    )
}

#[test]
fn nadir_records_the_cost_after_the_parent_reply() {
    let reply = nadir_reply(br#"{"choices":[],"nadir_metadata":{"cost":{"total_cost_usd":0.25}}}"#)
        .expect("the parent transforms the reply");
    assert_eq!(
        reply.response.get("choices"),
        Some(&Value::Array(Vec::new()))
    );
    let hidden = serde_json::to_value(&reply.hidden_params).unwrap();
    assert_eq!(
        hidden["additional_headers"][RESPONSE_COST_HEADER],
        Value::from(0.25_f64)
    );
}

#[test]
fn nadir_records_no_cost_when_the_bytes_carry_none() {
    let reply = nadir_reply(br#"{"choices":[],"nadir_metadata":{"cost":{"total_cost_usd":true}}}"#)
        .expect("the parent transforms the reply");
    assert!(reply.hidden_params.is_empty());
}

#[test]
fn nadir_returns_the_parent_error_before_reading_any_cost() {
    let result = nadir_reply(b"not json");
    assert!(matches!(result, Err(TransformResponseError::Json(_))));
}

#[rstest]
fn replays_the_nadir_cost_reply_corpus(#[files("fuzz/corpus/nadir_cost_reply/*")] path: PathBuf) {
    let body = fs::read(path).expect("the corpus fixture");
    let _cost = nadir_reported_cost(&body);
}
