#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tracing::subscriber::with_default;
use uuid::Uuid;

use super::capabilities::{CapabilityError, GroqCapabilities};
use super::credentials::compatible_provider_info;
use super::error::GroqChatError;
use super::messages::{MessageTransform, transform_messages};
use super::options::{
    MapParams, SupportedParams, get_supported_openai_params, is_compound_model, map_openai_params,
};
use super::reply::{BrowserActions, transform_response};
use super::stream::chunk_parser;
use crate::chat::ChatMessage;
use crate::chat_content::RefuseDownloads;
use crate::compatible_wire::ReplyOptions;
use crate::error::OpenAiError;
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

#[derive(Clone, Copy, Debug)]
enum Lookup {
    Yes,
    No,
    Fail,
}

struct Answers {
    web_search: Lookup,
    reasoning: Lookup,
    response_schema: Lookup,
}

impl Answers {
    const NO: Self = Self {
        web_search: Lookup::No,
        reasoning: Lookup::No,
        response_schema: Lookup::No,
    };
}

fn answer(lookup: Lookup, name: &'static str, model: &str) -> Result<bool, CapabilityError> {
    match lookup {
        Lookup::Yes => Ok(true),
        Lookup::No => Ok(false),
        Lookup::Fail => Err(CapabilityError {
            name,
            model: model.to_owned(),
            message: String::from("the catalog is unavailable"),
        }),
    }
}

impl GroqCapabilities for Answers {
    fn supports_web_search(&self, model: &str, _provider: &str) -> Result<bool, CapabilityError> {
        answer(self.web_search, "web search", model)
    }

    fn supports_reasoning(&self, model: &str, _provider: &str) -> Result<bool, CapabilityError> {
        answer(self.reasoning, "reasoning", model)
    }

    fn supports_response_schema(
        &self,
        model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityError> {
        answer(self.response_schema, "response schema", model)
    }
}

fn config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn supported(model: &str, answers: &Answers) -> Result<Vec<&'static str>, GroqChatError> {
    get_supported_openai_params(&SupportedParams {
        model,
        config: &config(),
        catalog: &catalog(),
        capabilities: answers,
    })
}

fn mapped(
    model: &str,
    non_default: &str,
    optional: &str,
    answers: &Answers,
) -> Result<super::options::MappedParams, GroqChatError> {
    map_openai_params(&MapParams {
        model,
        non_default: &object(non_default),
        optional_params: &object(optional),
        config: &config(),
        catalog: &catalog(),
        capabilities: answers,
        drop_params: false,
    })
}

fn captured<Action>(action: Action) -> String
where
    Action: FnOnce(),
{
    let log = tempfile::NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(log.reopen().expect("a reopened log"))
        .finish();
    with_default(subscriber, action);
    fs::read_to_string(log.path()).expect("the log")
}

fn reply(body: &Value) -> super::reply::GroqReply {
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: Uuid::nil,
        clock: i64::default,
    };
    transform_response(body.to_string().as_bytes(), &options).expect("the reply")
}

fn transformed(messages: &str) -> Value {
    let messages: Vec<ChatMessage> = serde_json::from_str(messages).expect("messages");
    let environment = BTreeMap::<String, String>::new();
    let downloads = RefuseDownloads;
    let transform = MessageTransform {
        model: "llama-3.3-70b-versatile",
        messages: &messages,
        environment: &environment,
        downloads: &downloads,
    };
    serde_json::to_value(transform_messages(&transform).expect("the transformed messages"))
        .expect("the message JSON")
}

fn billed(search: u64, opens: u64) -> Option<BrowserActions> {
    Some(BrowserActions {
        web_search_requests: search,
        browser_open_requests: opens,
    })
}

fn reply_with(executed: &Value) -> Value {
    json!({
        "id": "c", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": null, "executed_tools": executed}}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
    })
}

#[test]
fn credentials_prefer_explicit_then_environment_then_the_default_base() {
    let explicit_key = SecretString::from("explicit");
    let environment_key = SecretString::from("environment");
    let (base, key) = compatible_provider_info(
        Some("https://explicit.test"),
        Some(&explicit_key),
        Some("https://environment.test"),
        Some(&environment_key),
    );
    assert_eq!(base, "https://explicit.test");
    assert_eq!(
        key.as_ref().map(|key| key.expose_secret().as_str()),
        Some("explicit")
    );
    let (base, key) = compatible_provider_info(
        None,
        None,
        Some("https://environment.test"),
        Some(&environment_key),
    );
    assert_eq!(base, "https://environment.test");
    assert_eq!(
        key.as_ref().map(|key| key.expose_secret().as_str()),
        Some("environment")
    );
    let (base, key) = compatible_provider_info(None, None, None, None);
    assert_eq!(base, "https://api.groq.com/openai/v1");
    assert!(key.is_none());
}

#[test]
fn an_empty_string_counts_as_unset_at_every_step() {
    let empty = SecretString::from("");
    let environment_key = SecretString::from("environment");
    let (base, key) = compatible_provider_info(
        Some(""),
        Some(&empty),
        Some("https://environment.test"),
        Some(&environment_key),
    );
    assert_eq!(base, "https://environment.test");
    assert_eq!(
        key.as_ref().map(|key| key.expose_secret().as_str()),
        Some("environment")
    );
    let (base, key) = compatible_provider_info(Some(""), Some(&empty), Some(""), Some(&empty));
    assert_eq!(base, "https://api.groq.com/openai/v1");
    assert!(key.is_none());
}

#[rstest]
#[case::plain("compound")]
#[case::mini("compound-mini")]
#[case::prefixed("groq/compound")]
#[case::prefixed_mini("groq/compound-mini")]
fn compound_models_are_the_two_names_after_one_prefix(#[case] model: &str) {
    assert!(is_compound_model(model));
}

#[rstest]
#[case::other("llama-3.3-70b-versatile")]
#[case::twice_prefixed("groq/groq/compound")]
#[case::compound_suffix("groq/compound-mini-preview")]
fn other_models_are_not_compound(#[case] model: &str) {
    assert!(!is_compound_model(model));
}

#[test]
fn the_supported_list_drops_max_retries_and_the_web_search_option() {
    let params = supported("llama-3.3-70b-versatile", &Answers::NO).expect("the list");
    assert!(params.contains(&"temperature"));
    assert!(!params.contains(&"max_retries"));
    assert!(!params.contains(&"web_search_options"));
    assert!(!params.contains(&"reasoning_effort"));
    assert!(params.contains(&"response_format"));
}

#[rstest]
#[case::compound("groq/compound", Lookup::No)]
#[case::capability("llama-3.3-70b-versatile", Lookup::Yes)]
fn web_search_options_stay_for_compound_or_capable_models(
    #[case] model: &str,
    #[case] lookup: Lookup,
) {
    let answers = Answers {
        web_search: lookup,
        ..Answers::NO
    };
    let params = supported(model, &answers).expect("the list");
    assert!(params.contains(&"web_search_options"));
}

#[test]
fn reasoning_effort_is_appended_last_when_the_lookup_says_yes() {
    let answers = Answers {
        reasoning: Lookup::Yes,
        ..Answers::NO
    };
    let params = supported("llama-3.3-70b-versatile", &answers).expect("the list");
    assert_eq!(params.last(), Some(&"reasoning_effort"));
    assert_eq!(
        params
            .iter()
            .filter(|name| **name == "reasoning_effort")
            .count(),
        1
    );
}

#[test]
fn a_failed_reasoning_lookup_counts_as_no() {
    let answers = Answers {
        reasoning: Lookup::Fail,
        ..Answers::NO
    };
    let log = captured(|| {
        let params = supported("llama-3.3-70b-versatile", &answers).expect("the list");
        assert!(!params.contains(&"reasoning_effort"));
    });
    assert!(log.contains("reasoning capability lookup failed"));
}

#[test]
fn a_failed_web_search_lookup_propagates() {
    let answers = Answers {
        web_search: Lookup::Fail,
        ..Answers::NO
    };
    let error = supported("llama-3.3-70b-versatile", &answers).expect_err("the failure");
    assert_eq!(error.status_code(), 500);
}

#[test]
fn assistant_messages_lose_their_null_members_before_the_shared_transform() {
    let value = transformed(
        r#"[{"role":"assistant","content":null,"custom_null":null,"function_call":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]}]"#,
    );
    assert!(value[0].get("content").is_none());
    assert!(value[0].get("custom_null").is_none());
    assert!(value[0].get("function_call").is_none());
    assert_eq!(value[0]["tool_calls"][0]["id"], "c1");
}

#[test]
fn other_roles_keep_their_null_members() {
    let value = transformed(r#"[{"role":"user","content":"hi","custom_null":null}]"#);
    assert_eq!(value[0]["custom_null"], Value::Null);
}

#[test]
fn a_non_null_response_format_forces_a_fake_stream() {
    let mapped = mapped(
        "llama-3.3-70b-versatile",
        r#"{"response_format":{"type":"json_object"}}"#,
        "{}",
        &Answers::NO,
    )
    .expect("the mapping");
    assert_eq!(mapped.optional_params["fake_stream"], true);
    assert_eq!(
        mapped.optional_params["response_format"]["type"],
        "json_object"
    );
}

#[test]
fn a_null_response_format_does_not_force_a_fake_stream() {
    let mapped = mapped(
        "llama-3.3-70b-versatile",
        r#"{"response_format":null}"#,
        "{}",
        &Answers::NO,
    )
    .expect("the mapping");
    assert!(mapped.optional_params.get("fake_stream").is_none());
}

#[test]
fn an_unsupported_schema_becomes_a_single_json_tool_call() {
    let mapped = mapped(
        "llama-3.3-70b-versatile",
        r#"{"response_format":{"type":"json_schema","json_schema":{"schema":{"type":"object"}}}}"#,
        r#"{"temperature":0.1}"#,
        &Answers::NO,
    )
    .expect("the mapping");
    assert_eq!(
        mapped.optional_params["tools"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        mapped.optional_params["tools"][0]["function"]["name"],
        "json_tool_call"
    );
    assert_eq!(
        mapped.optional_params["tools"][0]["function"]["parameters"]["type"],
        "object"
    );
    assert_eq!(
        mapped.optional_params["tool_choice"]["function"]["name"],
        "json_tool_call"
    );
    assert_eq!(mapped.optional_params["json_mode"], true);
    assert_eq!(mapped.optional_params["fake_stream"], true);
    assert!(mapped.non_default.get("response_format").is_none());
    assert_eq!(mapped.optional_params["temperature"], 0.1);
}

#[test]
fn a_response_schema_key_wins_over_json_schema() {
    let mapped = mapped(
        "llama-3.3-70b-versatile",
        r#"{"response_format":{"type":"json_schema","response_schema":{"type":"array"},"json_schema":{"schema":{"type":"object"}}}}"#,
        "{}",
        &Answers::NO,
    )
    .expect("the mapping");
    assert_eq!(
        mapped.optional_params["tools"][0]["function"]["parameters"]["type"],
        "array"
    );
}

#[test]
fn a_native_schema_model_keeps_the_response_format() {
    let answers = Answers {
        response_schema: Lookup::Yes,
        ..Answers::NO
    };
    let mapped = mapped(
        "openai/gpt-oss-120b",
        r#"{"response_format":{"type":"json_schema","json_schema":{"schema":{"type":"object"}}}}"#,
        "{}",
        &answers,
    )
    .expect("the mapping");
    assert_eq!(
        mapped.optional_params["response_format"]["type"],
        "json_schema"
    );
    assert!(mapped.optional_params.get("tools").is_none());
    assert_eq!(mapped.optional_params["fake_stream"], true);
}

#[test]
fn a_schema_with_user_tools_fails_with_the_conflict_message() {
    let error = mapped(
        "llama-3.3-70b-versatile",
        r#"{"response_format":{"type":"json_schema","json_schema":{"schema":{"type":"object"}}},"tools":null}"#,
        "{}",
        &Answers::NO,
    )
    .expect_err("the conflict");
    assert_eq!(error.status_code(), 400);
    let rendered = error.to_string();
    assert!(rendered.contains(
        "Groq model 'llama-3.3-70b-versatile' does not support native structured outputs"
    ));
    assert!(rendered.contains("https://console.groq.com/docs/structured-outputs#supported-models"));
}

#[test]
fn max_completion_tokens_is_sent_as_max_tokens() {
    let mapped = mapped(
        "llama-3.3-70b-versatile",
        r#"{"max_completion_tokens":64}"#,
        "{}",
        &Answers::NO,
    )
    .expect("the mapping");
    assert_eq!(mapped.optional_params["max_tokens"], 64);
    assert!(
        mapped
            .optional_params
            .get("max_completion_tokens")
            .is_none()
    );
}

#[test]
fn web_search_options_add_one_browser_search_tool() {
    let mapped = mapped(
        "llama-3.3-70b-versatile",
        r#"{"web_search_options":{"search_context_size":"high"}}"#,
        "{}",
        &Answers::NO,
    )
    .expect("the mapping");
    assert_eq!(
        mapped.optional_params["tools"],
        json!([{"type": "browser_search"}])
    );
    assert!(mapped.non_default.get("web_search_options").is_none());
}

#[test]
fn an_existing_browser_search_tool_is_not_duplicated() {
    let mapped = mapped(
        "llama-3.3-70b-versatile",
        r#"{"web_search_options":{}}"#,
        r#"{"tools":[{"type":"browser_search"}]}"#,
        &Answers::NO,
    )
    .expect("the mapping");
    assert_eq!(
        mapped.optional_params["tools"].as_array().map(Vec::len),
        Some(1)
    );
}

#[test]
fn an_empty_web_search_options_object_still_adds_the_tool_without_a_log() {
    let log = captured(|| {
        let mapped = mapped(
            "llama-3.3-70b-versatile",
            r#"{"web_search_options":{}}"#,
            "{}",
            &Answers::NO,
        )
        .expect("the mapping");
        assert_eq!(
            mapped.optional_params["tools"],
            json!([{"type": "browser_search"}])
        );
    });
    assert!(!log.contains("Groq web search enabled"));
}

#[test]
fn a_non_empty_web_search_options_logs_its_sorted_keys() {
    let log = captured(|| {
        let mapped = mapped(
            "llama-3.3-70b-versatile",
            r#"{"web_search_options":{"search_context_size":"high","user_location":{}}}"#,
            "{}",
            &Answers::NO,
        )
        .expect("the mapping");
        assert_eq!(
            mapped.optional_params["tools"],
            json!([{"type": "browser_search"}])
        );
    });
    assert!(
        log.contains("Groq web search enabled; ignoring unsupported web_search_options fields")
    );
    assert!(log.contains("search_context_size"));
}

#[test]
fn a_compound_model_never_gets_the_browser_search_tool() {
    let mapped = mapped(
        "groq/compound",
        r#"{"web_search_options":{}}"#,
        "{}",
        &Answers::NO,
    )
    .expect("the mapping");
    assert!(mapped.optional_params.get("tools").is_none());
}

#[test]
fn a_null_web_search_options_adds_no_tool() {
    let mapped = mapped(
        "llama-3.3-70b-versatile",
        r#"{"web_search_options":null}"#,
        "{}",
        &Answers::NO,
    )
    .expect("the mapping");
    assert!(mapped.optional_params.get("tools").is_none());
    assert!(mapped.non_default.get("web_search_options").is_none());
}

#[test]
fn reasoning_effort_reaches_the_mapped_options_when_it_is_supported() {
    let answers = Answers {
        reasoning: Lookup::Yes,
        ..Answers::NO
    };
    let mapped = mapped(
        "llama-3.3-70b-versatile",
        r#"{"reasoning_effort":"high"}"#,
        "{}",
        &answers,
    )
    .expect("the mapping");
    assert_eq!(mapped.optional_params["reasoning_effort"], "high");
}

#[rstest]
#[case::absent("{}", "auto")]
#[case::null(r#"{"service_tier":null}"#, "auto")]
#[case::unknown(r#"{"service_tier":"scale"}"#, "auto")]
#[case::auto(r#"{"service_tier":"auto"}"#, "auto")]
#[case::default_tier(r#"{"service_tier":"default"}"#, "default")]
#[case::flex(r#"{"service_tier":"flex"}"#, "flex")]
fn service_tier_is_mapped_to_a_groq_supported_value(#[case] extra: &str, #[case] expected: &str) {
    let mut body = reply_with(&json!([]));
    let extra: Value = serde_json::from_str(extra).expect("extra fields");
    for (name, value) in extra.as_object().expect("an object") {
        body[name.as_str()] = value.clone();
    }
    let converted = reply(&body);
    assert_eq!(
        converted.reply.response.service_tier.as_deref(),
        Some(expected)
    );
}

#[test]
fn browser_searches_and_opens_are_counted_from_executed_tools() {
    let converted = reply(&reply_with(&json!([
        {"type": "browser_search"},
        {"name": "browser.search"},
        {"type": "browser.open"},
        {"type": "other"}
    ])));
    assert_eq!(converted.browser_actions, billed(2, 1));
}

#[test]
fn a_name_wins_over_the_type_of_an_executed_tool() {
    let converted = reply(&reply_with(&json!([
        {"name": "browser.open", "type": "browser_search"}
    ])));
    assert_eq!(converted.browser_actions, billed(0, 1));
}

#[rstest]
#[case::empty_list(r#"[]"#)]
#[case::unknown_actions(r#"[{"name": null, "type": "other"}]"#)]
#[case::no_action_when_both_are_empty(r#"[{"name": "", "type": ""}]"#)]
fn a_list_without_browser_actions_is_not_billed(#[case] executed: &str) {
    let converted = reply(&reply_with(
        &serde_json::from_str(executed).expect("a list"),
    ));
    assert!(converted.browser_actions.is_none());
}

#[rstest]
#[case::entry_not_an_object(r#"[3]"#)]
#[case::name_not_a_string(r#"[{"name": 3, "type": "browser_search"}]"#)]
#[case::type_not_a_string(r#"[{"name": null, "type": false}]"#)]
#[case::executed_tools_not_a_list(r#"{"name": "browser.search"}"#)]
fn a_malformed_executed_tools_list_bills_nothing_and_keeps_the_reply(#[case] executed: &str) {
    let body = reply_with(&serde_json::from_str(executed).expect("a JSON value"));
    let log = captured(|| {
        let converted = reply(&body);
        assert!(converted.browser_actions.is_none());
        assert_eq!(converted.reply.response.choices.len(), 1);
    });
    assert!(
        log.contains("Groq executed_tools entries did not match the expected shape; not billed")
    );
}

#[test]
fn a_reply_without_usage_is_not_billed() {
    let converted = reply(&json!({
        "id": "c", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": null, "executed_tools": [{"type": "browser_search"}]}}]
    }));
    assert!(converted.browser_actions.is_none());
}

#[test]
fn a_null_usage_is_not_billed() {
    let converted = reply(&json!({
        "id": "c", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": null, "executed_tools": [{"type": "browser_search"}]}}],
        "usage": null
    }));
    assert!(converted.browser_actions.is_none());
}

#[test]
fn an_error_chunk_fails_with_its_own_code_and_body() {
    let chunk = json!({"error": {"message": "rate limited", "code": 429}});
    let error = chunk_parser(&chunk, Uuid::nil, i64::default).expect_err("the provider error");
    assert_eq!(error.status_code(), 429);
    match error {
        GroqChatError::Provider(OpenAiError::Provider(error)) => {
            assert_eq!(error.message, "rate limited");
            assert_eq!(
                error.body.as_ref().and_then(|body| body.get("code")),
                Some(&json!(429))
            );
            assert!(!error.status_code_is_synthesized);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[rstest]
#[case::missing_code(json!({"error": {"message": "boom"}}))]
#[case::string_code(json!({"error": {"message": "boom", "code": "400"}}))]
#[case::null_code(json!({"error": {"message": "boom", "code": null}}))]
fn an_error_chunk_without_an_integer_code_uses_500(#[case] chunk: Value) {
    let error = chunk_parser(&chunk, Uuid::nil, i64::default).expect_err("the provider error");
    assert_eq!(error.status_code(), 500);
    match error {
        GroqChatError::Provider(OpenAiError::Provider(error)) => {
            assert!(error.status_code_is_synthesized);
            assert_eq!(error.message, "boom");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn an_integer_code_outside_the_base_range_is_passed_on() {
    // the base parser turns 200 into 500, while Groq keeps whatever integer the chunk carried
    let chunk = json!({"error": {"message": "odd", "code": 200}});
    let error = chunk_parser(&chunk, Uuid::nil, i64::default).expect_err("the provider error");
    assert_eq!(error.status_code(), 200);
}

#[test]
fn a_falsy_error_value_is_not_an_error() {
    let chunk = json!({"id": "c", "created": 1, "model": "m", "choices": [], "error": null});
    let parsed = chunk_parser(&chunk, Uuid::nil, i64::default).expect("a chunk");
    assert_eq!(parsed.object, "chat.completion.chunk");
}

#[test]
fn an_empty_name_falls_back_to_the_type() {
    let converted = reply(&reply_with(&json!([
        {"name": "", "type": "browser.open"}
    ])));
    assert_eq!(converted.browser_actions, billed(0, 1));
}

#[test]
fn a_zero_error_value_is_not_an_error() {
    let chunk = json!({"id": "c", "created": 1, "model": "m", "choices": [], "error": 0});
    let parsed = chunk_parser(&chunk, Uuid::nil, i64::default).expect("a chunk");
    assert_eq!(parsed.object, "chat.completion.chunk");
}

#[test]
fn delta_reasoning_becomes_reasoning_content() {
    let chunk = json!({
        "id": "c", "created": 1, "model": "m",
        "choices": [{"index": 0, "delta": {"reasoning": "thinking", "content": "answer"}}]
    });
    let parsed = chunk_parser(&chunk, Uuid::nil, i64::default).expect("a chunk");
    let delta = &parsed.choices[0].delta;
    assert_eq!(delta.reasoning_content.as_deref(), Some("thinking"));
    let value = serde_json::to_value(delta).expect("the delta JSON");
    assert!(value.get("reasoning").is_none());
}
