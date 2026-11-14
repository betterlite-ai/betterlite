#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::future::{self, Future};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::xai_identity::Settings as IdentitySettings;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use uuid::Uuid;

use super::error::XaiChatError;
use super::params::{ReasoningLookup, map_openai_params, supported_openai_params, supported_with};
use super::request::transform_request;
use super::response::{Reply, ReplyInput, transform_response};
use super::stream::parse_chunk;
use super::{
    DEFAULT_API_BASE, Environment, OauthTokens, PROVIDER, UrlInput, complete_url, provider_info,
    validate_environment,
};
use crate::chat::response::ChatUsage;
use crate::chat::{ChatMessage, MessageContent, Role};
use crate::chat_content::{ChatContent, RefuseDownloads};
use crate::openai_base_config::Headers;
use crate::xai_tokens::error::XaiOauthError;

const EXPECTED_BASE: [&str; 18] = [
    "logit_bias",
    "logprobs",
    "max_completion_tokens",
    "max_tokens",
    "n",
    "parallel_tool_calls",
    "presence_penalty",
    "response_format",
    "seed",
    "stream",
    "stream_options",
    "temperature",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "user",
    "web_search_options",
];

fn settings() -> IdentitySettings {
    IdentitySettings {
        xai_key: None,
        generic_api_key: None,
        environment_key: None,
        api_base: None,
    }
}

fn secret(text: &str) -> SecretString {
    SecretString::from(String::from(text))
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[derive(Clone)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    fn count(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner> Layer<Inner> for Warnings
where
    Inner: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

struct Token(&'static str);

impl OauthTokens for Token {
    fn api_base(&self) -> String {
        String::from("https://oauth.example/v1")
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send {
        let token = self.0;
        async move { Ok(secret(token)) }
    }
}

struct LoginRequired;

impl OauthTokens for LoginRequired {
    fn api_base(&self) -> String {
        String::from("https://oauth.example/v1")
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send {
        future::ready(Err(XaiOauthError::LoginRequired))
    }
}

struct Scripted(Option<bool>);

impl ReasoningLookup<io::Error> for Scripted {
    fn supports_reasoning(
        &mut self,
        _model: &str,
    ) -> impl Future<Output = Result<bool, io::Error>> + Send {
        let outcome = self.0;
        async move { outcome.ok_or_else(|| io::Error::other("the capability lookup failed")) }
    }
}

#[test]
fn stop_and_frequency_penalty_follow_the_model_substrings() {
    let mut plain = EXPECTED_BASE.to_vec();
    plain.extend(["stop", "frequency_penalty", "reasoning_effort"]);
    assert_eq!(supported_with("grok-2-1212", true), plain);
    let mut mini = EXPECTED_BASE.to_vec();
    mini.extend(["frequency_penalty", "reasoning_effort"]);
    assert_eq!(supported_with("grok-3-mini", true), mini);
    let mut four = EXPECTED_BASE.to_vec();
    four.push("reasoning_effort");
    assert_eq!(supported_with("grok-4", true), four);
    assert_eq!(supported_with("grok-code-fast-1", true), four);
    let mut without_reasoning = EXPECTED_BASE.to_vec();
    without_reasoning.extend(["stop", "frequency_penalty"]);
    assert_eq!(supported_with("grok-2-1212", false), without_reasoning);
}

#[tokio::test]
async fn reasoning_effort_follows_the_capability_lookup() {
    let mut supported = Scripted(Some(true));
    let with_reasoning = supported_openai_params("grok-4", &mut supported).await;
    assert!(with_reasoning.ends_with(&["reasoning_effort"]));
    let mut unsupported = Scripted(Some(false));
    let without_reasoning = supported_openai_params("grok-4", &mut unsupported).await;
    assert!(!without_reasoning.contains(&"reasoning_effort"));
    let mut failing = Scripted(None);
    let after_failure = supported_openai_params("grok-4", &mut failing).await;
    assert!(!after_failure.contains(&"reasoning_effort"));
    assert_eq!(after_failure, supported_with("grok-4", false));
}

#[test]
fn map_writes_max_completion_tokens_as_max_tokens() {
    let mut optional = Object::new();
    map_openai_params(
        "grok-4",
        false,
        &object(r#"{"max_completion_tokens":5}"#),
        &mut optional,
    )
    .expect("a mapped request");
    assert_eq!(
        optional.get("max_tokens"),
        Some(&Json::Number(serde_json::Number::from(5_i64)))
    );
    let mut nulled = Object::new();
    map_openai_params(
        "grok-4",
        false,
        &object(r#"{"max_completion_tokens":null}"#),
        &mut nulled,
    )
    .expect("a mapped request");
    assert_eq!(nulled.get("max_tokens"), Some(&Json::Null));
}

#[test]
fn tools_lose_strict_at_every_depth() {
    let requested = object(
        r#"{"tools":[{"type":"function","strict":true,"function":{"name":"f","strict":false,"parameters":{"type":"object","properties":{"a":{"type":"string","strict":true}}}}}]}"#,
    );
    let mut optional = Object::new();
    map_openai_params("grok-4", false, &requested, &mut optional).expect("a mapped request");
    let Some(Json::List(tools)) = optional.get("tools") else {
        panic!("the tools list is missing");
    };
    assert_eq!(
        serde_json::to_string(tools).expect("JSON"),
        r#"[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{"a":{"type":"string"}}}}}]"#
    );
}

#[test]
fn empty_cleaned_tools_are_omitted() {
    let mut optional = object(r#"{"tools":[{"type":"function"}]}"#);
    map_openai_params("grok-4", false, &object(r#"{"tools":[]}"#), &mut optional)
        .expect("a mapped request");
    assert_eq!(optional, object(r#"{"tools":[{"type":"function"}]}"#));
    let mut nulled = Object::new();
    map_openai_params("grok-4", false, &object(r#"{"tools":null}"#), &mut nulled)
        .expect("a mapped request");
    assert!(nulled.is_empty());
}

#[rstest]
#[case::not_a_list(r#"{"tools":{}}"#)]
#[case::entry_not_an_object(r#"{"tools":[1]}"#)]
fn non_object_tools_are_refused(#[case] requested: &str) {
    let mut optional = Object::new();
    let error = map_openai_params("grok-4", false, &object(requested), &mut optional)
        .expect_err("a refused request");
    assert_eq!(error.status_code(), 400);
}

#[test]
fn supported_nulls_and_unknown_names_are_not_inserted() {
    let requested =
        object(r#"{"temperature":null,"stream":true,"thinking":"on","drop_params":true}"#);
    let mut optional = Object::new();
    map_openai_params("grok-4", false, &requested, &mut optional).expect("a mapped request");
    assert_eq!(optional, object(r#"{"stream":true}"#));
}

fn content<'request>(
    messages: &'request [ChatMessage],
    optional: &'request Object,
    environment: &'request BTreeMap<String, String>,
) -> ChatContent<'request> {
    ChatContent {
        model: "grok-4",
        messages,
        optional_params: optional,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment,
        downloads: &RefuseDownloads,
    }
}

fn named(role: Role, name: &str) -> ChatMessage {
    ChatMessage {
        role,
        content: Some(MessageContent::Text(String::from("hi"))),
        name: Some(String::from(name)),
        tool_calls: None,
        tool_call_id: None,
        passthrough: indexmap::IndexMap::new(),
    }
}

#[test]
fn names_are_stripped_for_every_role_but_user() {
    let messages = [named(Role::User, "alice"), named(Role::Assistant, "bot")];
    let optional = Object::new();
    let environment = BTreeMap::new();
    let body = transform_request(&content(&messages, &optional, &environment))
        .expect("a transformed request");
    let Some(Json::List(rendered)) = body.get("messages") else {
        panic!("the messages are missing");
    };
    let Some(Json::Object(user)) = rendered.first() else {
        panic!("the user message is missing");
    };
    assert_eq!(user.get("name"), Some(&Json::Text(String::from("alice"))));
    let Some(Json::Object(assistant)) = rendered.get(1) else {
        panic!("the assistant message is missing");
    };
    assert!(!assistant.contains_key("name"));
}

#[rstest]
#[case::value(r#"{"web_search_options":{"search_context_size":"low"}}"#)]
#[case::null(r#"{"web_search_options":null}"#)]
fn web_search_options_warns_and_is_removed(#[case] optional: &str) {
    let mut parsed: Object = serde_json::from_str(optional).expect("an object");
    parsed.insert(
        String::from("temperature"),
        Json::Number(serde_json::Number::from(1_i64)),
    );
    let messages = [named(Role::User, "alice")];
    let environment = BTreeMap::new();
    let warnings = Warnings(Arc::default());
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let body = with_default(subscriber, || {
        transform_request(&content(&messages, &parsed, &environment))
    })
    .expect("a transformed request");
    assert!(!body.contains_key("web_search_options"));
    assert_eq!(
        body.get("temperature"),
        Some(&Json::Number(serde_json::Number::from(1_i64)))
    );
    assert_eq!(warnings.count(), 1);
}

#[tokio::test]
async fn oauth_is_skipped_when_a_key_resolves() {
    let mut settings = settings();
    settings.xai_key = Some(secret("xai-key"));
    settings.generic_api_key = Some(secret("generic"));
    let mut oauth = LoginRequired;
    let resolved = validate_environment(Environment {
        headers: Headers::new(),
        api_key: None,
        settings: &settings,
        model: "grok-4",
        use_xai_oauth: true,
        oauth: &mut oauth,
    })
    .await
    .expect("a validated environment");
    assert_eq!(
        resolved.get("Authorization"),
        Some(&String::from("Bearer xai-key"))
    );
    assert_eq!(
        resolved.get("Content-Type"),
        Some(&String::from("application/json"))
    );
}

#[rstest]
#[case::lowercase(&[("content-type", "text/plain")], "content-type", "text/plain")]
#[case::capitalized(&[("Content-Type", "text/plain")], "Content-Type", "text/plain")]
#[case::absent(&[], "Content-Type", "application/json")]
#[case::stale_authorization(&[("Authorization", "Bearer stale")], "Content-Type", "application/json")]
#[tokio::test]
async fn oauth_replaces_authorization_and_keeps_content_type_casing(
    #[case] pairs: &[(&str, &str)],
    #[case] name: &str,
    #[case] value: &str,
) {
    let settings = settings();
    let mut oauth = Token("oauth-token");
    let resolved = validate_environment(Environment {
        headers: headers(pairs),
        api_key: None,
        settings: &settings,
        model: "grok-4",
        use_xai_oauth: true,
        oauth: &mut oauth,
    })
    .await
    .expect("a validated environment");
    assert_eq!(
        resolved.get("Authorization"),
        Some(&String::from("Bearer oauth-token"))
    );
    assert_eq!(resolved.get(name), Some(&String::from(value)));
    assert_eq!(resolved.len(), 2);
}

#[tokio::test]
async fn oauth_failures_become_authentication_errors() {
    let settings = settings();
    let mut oauth = LoginRequired;
    let error = validate_environment(Environment {
        headers: Headers::new(),
        api_key: None,
        settings: &settings,
        model: "grok-4",
        use_xai_oauth: true,
        oauth: &mut oauth,
    })
    .await
    .expect_err("a refused environment");
    assert!(matches!(error, XaiChatError::Authentication { .. }));
    assert_eq!(error.status_code(), 401);
}

#[test]
fn the_chat_base_is_explicit_then_environment_then_default() {
    let mut settings = settings();
    assert_eq!(
        provider_info(Some("https://explicit/v1"), None, &settings).api_base,
        "https://explicit/v1"
    );
    assert_eq!(
        provider_info(None, None, &settings).api_base,
        DEFAULT_API_BASE
    );
    settings.api_base = Some(String::from("https://environment/v1"));
    assert_eq!(
        provider_info(None, None, &settings).api_base,
        "https://environment/v1"
    );
    assert_eq!(
        provider_info(Some(""), None, &settings).api_base,
        String::new()
    );
}

#[test]
fn the_chat_key_order_ignores_the_generic_key() {
    let mut settings = settings();
    settings.generic_api_key = Some(secret("generic"));
    settings.environment_key = Some(secret("environment"));
    let from_environment = provider_info(None, None, &settings).api_key;
    assert_eq!(
        from_environment.as_ref().map(ExposeSecret::expose_secret),
        Some("environment")
    );
    settings.xai_key = Some(secret("xai"));
    let from_xai = provider_info(None, None, &settings).api_key;
    assert_eq!(
        from_xai.as_ref().map(ExposeSecret::expose_secret),
        Some("xai")
    );
    let explicit = provider_info(None, Some(&secret("explicit")), &settings).api_key;
    assert_eq!(
        explicit.as_ref().map(ExposeSecret::expose_secret),
        Some("explicit")
    );
}

#[test]
fn oauth_replaces_the_base_only_without_a_key() {
    let settings = settings();
    let oauth = Token("oauth-token");
    let swapped = complete_url(&UrlInput {
        api_base: Some("https://explicit/v1"),
        api_key: None,
        settings: &settings,
        use_xai_oauth: true,
        oauth: &oauth,
    });
    assert_eq!(swapped, "https://oauth.example/v1/chat/completions");
    let keyed = complete_url(&UrlInput {
        api_base: Some("https://explicit/v1"),
        api_key: Some(&secret("key")),
        settings: &settings,
        use_xai_oauth: true,
        oauth: &oauth,
    });
    assert_eq!(keyed, "https://explicit/v1/chat/completions");
    let plain = complete_url(&UrlInput {
        api_base: Some("https://explicit/v1/chat/completions"),
        api_key: None,
        settings: &settings,
        use_xai_oauth: false,
        oauth: &oauth,
    });
    assert_eq!(plain, "https://explicit/v1/chat/completions");
}

fn reply(body: &str) -> Reply {
    transform_response(&ReplyInput {
        body: body.as_bytes(),
    })
    .expect("a converted reply")
}

const TOOL_REPLY: &str = r#"{"id":"c","object":"chat.completion","created":1,"model":"grok-4","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{}"}}]},"finish_reason":""}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":17,"completion_tokens_details":{"reasoning_tokens":5},"cost_in_usd_ticks":123,"server_side_tool_usage_details":{"web_search_calls":2}}}"#;

const STOP_WITH_CALLS: &str = r#"{"id":"c","object":"chat.completion","created":1,"model":"grok-4","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{}"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

#[test]
fn an_empty_finish_reason_is_mapped_to_stop_by_the_parent_before_the_repair() {
    assert_eq!(
        reply(TOOL_REPLY).response.choices[0]
            .finish_reason
            .as_deref(),
        Some("stop")
    );
    assert_eq!(
        reply(TOOL_REPLY).response.choices[0].provider_specific_fields,
        BTreeMap::from([(String::from("native_finish_reason"), Value::from(""))])
    );
    let without_calls = reply(
        r#"{"id":"c","object":"chat.completion","created":1,"model":"grok-4","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":""}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
    );
    assert_eq!(
        without_calls.response.choices[0].finish_reason.as_deref(),
        Some("stop")
    );
}

#[test]
fn a_stop_finish_with_tool_calls_becomes_tool_calls() {
    assert_eq!(
        reply(STOP_WITH_CALLS).response.choices[0]
            .finish_reason
            .as_deref(),
        Some("tool_calls")
    );
    let length = reply(&STOP_WITH_CALLS.replace("\"stop\"}],", "\"length\"}],"));
    assert_eq!(
        length.response.choices[0].finish_reason.as_deref(),
        Some("length")
    );
}

#[rstest]
#[case::null(r#","finish_reason":null"#)]
#[case::absent("")]
fn a_tool_call_reply_without_a_finish_reason_is_tool_calls(#[case] finish: &str) {
    let body = STOP_WITH_CALLS.replace(r#","finish_reason":"stop""#, finish);
    assert_eq!(
        reply(&body).response.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
}

#[rstest]
#[case::null(r#","finish_reason":null"#)]
#[case::absent("")]
fn a_reply_without_a_finish_reason_or_calls_is_stop(#[case] finish: &str) {
    let body = format!(
        r#"{{"id":"c","object":"chat.completion","created":1,"model":"grok-4","choices":[{{"index":0,"message":{{"role":"assistant","content":"hi"}}{finish}}}],"usage":{{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}}}"#
    );
    let converted = reply(&body);
    assert_eq!(
        converted.response.choices[0].finish_reason.as_deref(),
        Some("stop")
    );
    assert!(
        converted.response.choices[0]
            .provider_specific_fields
            .is_empty()
    );
}

#[test]
fn a_null_usage_block_is_answered_with_zero_usage() {
    let converted = reply(include_str!(
        "../../fuzz/corpus/xai_chat_reply/null-usage-no-choices.json"
    ));
    assert_eq!(converted.response.usage, ChatUsage::default());
    assert_eq!(converted.usage.tokens.total_tokens, 0);
    assert_eq!(converted.usage.cost, None);
}

#[test]
fn reply_usage_is_folded_normalized_and_restated() {
    let converted = reply(TOOL_REPLY);
    assert_eq!(converted.usage.tokens.prompt_tokens, 10);
    assert_eq!(converted.usage.tokens.completion_tokens, 7);
    assert_eq!(converted.usage.tokens.total_tokens, 17);
    assert_eq!(
        converted.usage.cost,
        Some("0.0000000123".parse::<Decimal>().expect("a decimal"))
    );
    assert_eq!(
        converted
            .usage
            .tokens
            .prompt_details
            .and_then(|details| details.web_search_requests),
        Some(2)
    );
    assert_eq!(
        converted
            .usage
            .server_side_tool_usage_details
            .as_ref()
            .and_then(|details| details.get("web_search_calls")),
        Some(&Value::from(2_i64))
    );
    assert_eq!(converted.response.usage.completion_tokens, 7);
    assert_eq!(converted.response.usage.total_tokens, 17);
    assert_eq!(
        converted
            .response
            .usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|details| details.get("web_search_requests")),
        Some(&Some(2))
    );
}

#[test]
fn a_reply_with_malformed_usage_extras_is_retained() {
    let converted = reply(
        r#"{"id":"c","object":"chat.completion","created":1,"model":"grok-4","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":17,"completion_tokens_details":{"reasoning_tokens":5},"server_side_tool_usage_details":7}}"#,
    );
    assert_eq!(
        converted.response.choices[0].finish_reason.as_deref(),
        Some("stop")
    );
    assert_eq!(converted.usage.tokens.completion_tokens, 7);
    assert_eq!(converted.usage.tokens.total_tokens, 17);
    assert_eq!(converted.usage.cost, None);
    assert!(converted.usage.server_side_tool_usage_details.is_none());
}

#[rstest]
fn replays_the_xai_chat_reply_corpus(#[files("fuzz/corpus/xai_chat_reply/*")] path: PathBuf) {
    let body = fs::read(path).unwrap();
    let _converted = transform_response(&ReplyInput { body: &body });
}

fn zero_clock() -> i64 {
    0
}

fn chunk(text: &str) -> Value {
    serde_json::from_str(text).expect("a chunk")
}

#[test]
fn a_null_usage_chunk_gains_one_empty_choice() {
    let mut parsed = chunk(
        r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[],"usage":null}"#,
    );
    let reply = parse_chunk(&mut parsed, Uuid::nil, zero_clock).expect("a parsed chunk");
    assert_eq!(reply.choices.len(), 1);
    assert_eq!(reply.choices[0].index, 0);
    assert_eq!(reply.choices[0].finish_reason, None);
    assert_eq!(reply.usage, None);
    assert_eq!(parsed["choices"].as_array().map(Vec::len), Some(1));
    let choice = &parsed["choices"][0];
    assert_eq!(choice["index"].as_i64(), Some(0));
    assert_eq!(choice["delta"].as_object().map(Map::len), Some(0));
    assert!(choice["finish_reason"].is_null());
}

#[test]
fn streamed_usage_is_reconciled_before_the_parent_parser() {
    let mut parsed = chunk(
        r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":17,"completion_tokens_details":{"reasoning_tokens":5},"cost_in_usd_ticks":123}}"#,
    );
    let reply = parse_chunk(&mut parsed, Uuid::nil, zero_clock).expect("a parsed chunk");
    assert_eq!(reply.choices.len(), 1);
    let Some(Value::Object(usage)) = reply.usage else {
        panic!("the usage is missing");
    };
    assert_eq!(usage.get("prompt_tokens"), Some(&Value::from(10_i64)));
    assert_eq!(usage.get("completion_tokens"), Some(&Value::from(7_i64)));
    assert_eq!(usage.get("total_tokens"), Some(&Value::from(17_i64)));
    assert_eq!(
        usage.get("cost").map(ToString::to_string),
        Some(String::from("0.0000000123"))
    );
}

#[test]
fn a_chunk_with_usage_but_no_ticks_gets_no_cost() {
    let mut parsed = chunk(
        r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
    );
    let reply = parse_chunk(&mut parsed, Uuid::nil, zero_clock).expect("a parsed chunk");
    assert_eq!(reply.choices.len(), 1);
    assert_eq!(reply.choices[0].finish_reason.as_deref(), Some("stop"));
    let Some(Value::Object(usage)) = reply.usage else {
        panic!("the usage is missing");
    };
    assert!(!usage.contains_key("cost"));
}

#[test]
fn a_chunk_without_a_usage_key_keeps_its_empty_choices() {
    let mut parsed = chunk(
        r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[]}"#,
    );
    let reply = parse_chunk(&mut parsed, Uuid::nil, zero_clock).expect("a parsed chunk");
    assert!(reply.choices.is_empty());
    assert_eq!(parsed["choices"].to_string(), "[]");
}
