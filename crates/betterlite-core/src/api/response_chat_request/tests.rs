#![cfg(test)]

use std::fmt::Debug;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::{Map, Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::records::ChatRequestInput;
use super::request::build_request;
use super::tools::add_web_search_tool;
use crate::api::response_chat_history::history::HistoryMessage;

const USER: &str = r#"[{"role": "user", "content": "hello"}]"#;
const SYSTEM_AND_USER: &str =
    r#"[{"role": "system", "content": "rules"}, {"role": "user", "content": "hi"}]"#;
const EMPTY_SYSTEM_AND_USER: &str =
    r#"[{"role": "system", "content": ""}, {"role": "user", "content": "hi"}]"#;
const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Logs {
    fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
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

struct Projection {
    params: Value,
    litellm_params: Value,
    extra_headers: Value,
    client: Value,
}

struct Built {
    outcome: Result<Projection, String>,
    optional_params: Value,
    call_type: String,
    logs: Vec<String>,
}

fn map(source: &str) -> IndexMap<String, Value> {
    serde_json::from_str(source).expect("the fixture is a JSON object")
}

fn string_map(source: &str) -> IndexMap<String, String> {
    serde_json::from_str(source).expect("the fixture is a JSON object of strings")
}

fn chat_messages(source: &str) -> Vec<HistoryMessage> {
    serde_json::from_str(source).expect("the fixture is chat messages")
}

fn logging_state() -> LoggingState {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor");
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).expect("the logging init"),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .expect("a logging state")
}

fn build_with(
    messages: &str,
    optional: &str,
    litellm: &str,
    headers: &str,
    auto_summary: bool,
) -> Built {
    let mut state = logging_state();
    let mut optional_params = map(optional);
    let litellm_params = map(litellm);
    let headers = string_map(headers);
    let history = chat_messages(messages);
    let logs = Logs::default();
    let subscriber = registry().with(logs.clone());
    let outcome = with_default(subscriber, || {
        build_request(
            ChatRequestInput {
                model: "gpt-4o",
                messages: &history,
                optional_params: &mut optional_params,
                litellm_params: &litellm_params,
                headers: &headers,
                reasoning_auto_summary: auto_summary,
            },
            &mut state,
            Some("marker"),
        )
        .map(|request| Projection {
            params: serde_json::to_value(&request.params).expect("the request serializes"),
            litellm_params: serde_json::to_value(&request.litellm_params)
                .expect("the SDK params serialize"),
            extra_headers: serde_json::to_value(&request.extra_headers)
                .expect("the headers serialize"),
            client: serde_json::to_value(request.client).expect("the client serializes"),
        })
        .map_err(|error| error.to_string())
    });
    Built {
        outcome,
        optional_params: serde_json::to_value(&optional_params).expect("the options serialize"),
        call_type: state.call_type,
        logs: logs.take(),
    }
}

fn build(optional: &str) -> Built {
    build_with(USER, optional, "{}", "{}", false)
}

fn options(params: &Value) -> Value {
    let mut options = params.clone();
    let object = options.as_object_mut().expect("the request is an object");
    object.remove("model");
    object.remove("input");
    options
}

#[rstest]
#[case::max_completion_tokens_last(
    r#"{"max_tokens": 4, "max_completion_tokens": 7}"#,
    json!({"max_output_tokens": 7})
)]
#[case::max_tokens_last(
    r#"{"max_completion_tokens": 7, "max_tokens": 4}"#,
    json!({"max_output_tokens": 4})
)]
#[case::max_output_tokens_direct(
    r#"{"max_tokens": 4, "max_output_tokens": 9, "max_completion_tokens": 3}"#,
    json!({"max_output_tokens": 3})
)]
#[case::nulls_skipped(r#"{"temperature": null, "max_tokens": null}"#, json!({}))]
#[case::unknown_keys_dropped(
    r#"{"vendor": {"x": 1}, "functions": [], "max_tokens": 4}"#,
    json!({"max_output_tokens": 4})
)]
#[case::declared_keys_forwarded(
    r#"{"include": ["reasoning.encrypted_content"], "store": false, "parallel_tool_calls": true, "prompt_cache_key": "k", "top_logprobs": 3, "context_management": [{"type": "compaction"}]}"#,
    json!({
        "include": ["reasoning.encrypted_content"],
        "store": false,
        "parallel_tool_calls": true,
        "prompt_cache_key": "k",
        "top_logprobs": 3,
        "context_management": [{"type": "compaction"}]
    })
)]
#[case::stream_options_normalized(
    r#"{"stream_options": {"include_usage": true, "include_obfuscation": false}}"#,
    json!({"stream_options": {"include_obfuscation": false}})
)]
#[case::stream_options_without_obfuscation_dropped(
    r#"{"stream_options": {"include_usage": true}}"#,
    json!({})
)]
#[case::tool_choice_flat_passthrough(r#"{"tool_choice": "auto"}"#, json!({"tool_choice": "auto"}))]
#[case::tool_choice_nested_function(
    r#"{"tool_choice": {"type": "function", "function": {"name": "f"}}}"#,
    json!({"tool_choice": {"type": "function", "name": "f"}})
)]
fn maps_every_option_key(#[case] optional: &str, #[case] expected: Value) {
    let built = build(optional);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(options(&projection.params), expected);
}

#[rstest]
#[case::verbosity_then_schema(
    r#"{"verbosity": "low", "response_format": {"type": "json_schema", "json_schema": {}}}"#
)]
#[case::schema_then_verbosity(
    r#"{"response_format": {"type": "json_schema", "json_schema": {}}, "verbosity": "low"}"#
)]
fn merges_verbosity_with_a_json_schema(#[case] optional: &str) {
    let built = build(optional);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(
        options(&projection.params),
        json!({
            "text": {
                "verbosity": "low",
                "format": {
                    "type": "json_schema",
                    "name": "response_schema",
                    "schema": {},
                    "strict": false
                }
            }
        })
    );
}

#[rstest]
#[case::json_object(
    r#"{"response_format": {"type": "json_object"}}"#,
    json!({"text": {"format": {"type": "json_object"}}})
)]
#[case::text(r#"{"response_format": {"type": "text"}}"#, json!({"text": {"format": {"type": "text"}}}))]
#[case::unknown_type(r#"{"response_format": {"type": "vendor"}}"#, json!({}))]
#[case::empty_object(r#"{"response_format": {}}"#, json!({}))]
#[case::non_object(r#"{"response_format": "json"}"#, json!({}))]
#[case::explicit_nulls_kept(
    r#"{"response_format": {"type": "json_schema", "json_schema": {"name": null, "schema": null, "strict": null}}}"#,
    json!({"text": {"format": {"type": "json_schema", "name": null, "schema": null, "strict": null}}})
)]
#[case::named_schema(
    r#"{"response_format": {"type": "json_schema", "json_schema": {"name": "out", "schema": {"type": "object"}, "strict": true}}}"#,
    json!({"text": {"format": {"type": "json_schema", "name": "out", "schema": {"type": "object"}, "strict": true}}})
)]
fn maps_response_formats(#[case] optional: &str, #[case] expected: Value) {
    let built = build(optional);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(options(&projection.params), expected);
}

#[rstest]
#[case::existing_fields_survive(
    r#"{"text": {"verbosity": "high", "extra": 1}, "verbosity": "low"}"#,
    json!({"text": {"verbosity": "low", "extra": 1}})
)]
#[case::later_text_replaces(
    r#"{"verbosity": "low", "text": {"format": {"type": "text"}}}"#,
    json!({"text": {"format": {"type": "text"}}})
)]
#[case::empty_text_merges(r#"{"text": {}, "verbosity": "low"}"#, json!({"text": {"verbosity": "low"}}))]
fn keeps_text_across_the_mapping(#[case] optional: &str, #[case] expected: Value) {
    let built = build(optional);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(options(&projection.params), expected);
}

#[test]
fn truncates_a_long_user_to_64_characters() {
    let sixty_four = "u".repeat(64);
    let sixty_five = "u".repeat(65);
    let built = build(&format!(r#"{{"user": "{sixty_four}"}}"#));
    let projection = built.outcome.expect("the request builds");
    assert_eq!(options(&projection.params)["user"], json!(sixty_four));
    let built = build(&format!(r#"{{"user": "{sixty_five}"}}"#));
    let projection = built.outcome.expect("the request builds");
    assert_eq!(options(&projection.params)["user"], json!(sixty_four));
    let built = build(&format!(r#"{{"user": "{}"}}"#, "é".repeat(65)));
    let projection = built.outcome.expect("the request builds");
    assert_eq!(options(&projection.params)["user"], json!("é".repeat(64)));
}

#[rstest]
#[case::temperature(r#"{"temperature": "hot"}"#, "temperature")]
#[case::user(r#"{"user": 7}"#, "user")]
#[case::tools_not_a_list(r#"{"tools": "x"}"#, "tools")]
#[case::tool_without_name(r#"{"tools": [{"type": "function", "function": {}}]}"#, "tools")]
#[case::custom_tool_null_name(
    r#"{"tools": [{"type": "custom", "custom": {"name": null}}]}"#,
    "tools"
)]
#[case::extra_body(r#"{"extra_body": "x"}"#, "extra_body")]
#[case::json_schema(
    r#"{"response_format": {"type": "json_schema", "json_schema": null}}"#,
    "response_format"
)]
#[case::text_merge(r#"{"text": "x", "verbosity": "low"}"#, "text")]
#[case::stream(r#"{"stream": "yes"}"#, "stream")]
fn rejects_values_of_the_wrong_type(#[case] optional: &str, #[case] field: &str) {
    let built = build(optional);
    let message = built.outcome.expect_err("the request is rejected");
    assert!(message.contains(field), "{message}");
}

#[test]
fn rejects_sdk_extra_headers_that_are_not_a_map() {
    let built = build_with(USER, "{}", r#"{"extra_headers": "x"}"#, "{}", false);
    let message = built.outcome.expect_err("the request is rejected");
    assert!(message.contains("extra_headers"), "{message}");
}

#[rstest]
#[case::text(r#"[{"role": "system", "content": "rules"}]"#, "rules")]
#[case::empty_string(r#"[{"role": "system", "content": ""}]"#, "")]
fn carries_a_system_only_chat_as_input(#[case] messages: &str, #[case] text: &str) {
    let built = build_with(messages, "{}", "{}", "{}", false);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(
        projection.params["input"],
        json!([{
            "type": "message",
            "role": "system",
            "content": [{"type": "input_text", "text": text}]
        }])
    );
    assert!(projection.params.get("instructions").is_none());
}

#[rstest]
#[case::history_wins(SYSTEM_AND_USER, r#"{"instructions": "other"}"#, json!({"instructions": "rules"}))]
#[case::option_without_history(USER, r#"{"instructions": "other"}"#, json!({"instructions": "other"}))]
#[case::empty_history_keeps_option(
    EMPTY_SYSTEM_AND_USER,
    r#"{"instructions": "other"}"#,
    json!({"instructions": "other"})
)]
fn prefers_history_instructions(
    #[case] messages: &str,
    #[case] optional: &str,
    #[case] expected: Value,
) {
    let built = build_with(messages, optional, "{}", "{}", false);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(options(&projection.params), expected);
}

#[rstest]
#[case::default_false("{}", "{}", json!({}), "False")]
#[case::option_true(r#"{"stream": true}"#, "{}", json!({"stream": true}), "True")]
#[case::option_false_forwarded(r#"{"stream": false}"#, "{}", json!({"stream": false}), "False")]
#[case::sdk_true_beats_option_false(
    r#"{"stream": false}"#,
    r#"{"stream": true}"#,
    json!({"stream": true}),
    "True"
)]
#[case::sdk_true_alone("{}", r#"{"stream": true}"#, json!({"stream": true}), "True")]
#[case::null_option_falls_back(
    r#"{"stream": null}"#,
    r#"{"stream": true}"#,
    json!({"stream": true}),
    "True"
)]
fn resolves_stream(
    #[case] optional: &str,
    #[case] litellm: &str,
    #[case] expected: Value,
    #[case] logged: &str,
) {
    let built = build_with(USER, optional, litellm, "{}", false);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(options(&projection.params), expected);
    let message = format!("Chat provider: Stream parameter: {logged}");
    assert!(built.logs.contains(&message), "{:?}", built.logs);
}

#[test]
fn logs_the_builder_diagnostics_in_order() {
    let built = build_with(
        SYSTEM_AND_USER,
        r#"{"previous_response_id": "resp_a", "stream": true}"#,
        "{}",
        "{}",
        false,
    );
    assert_eq!(
        built.logs,
        [
            "Chat provider: Stream parameter: True",
            "Chat provider: Warning ignoring previous response ID: resp_a",
            "Chat provider: Final request model=gpt-4o, input_items=1",
        ]
    );
    let projection = built.outcome.expect("the request builds");
    assert_eq!(projection.params["previous_response_id"], json!("resp_a"));
}

#[test]
fn skips_the_previous_response_warning_for_an_empty_id() {
    let built = build(r#"{"previous_response_id": ""}"#);
    assert!(
        !built
            .logs
            .iter()
            .any(|line| line.contains("Warning ignoring previous response ID")),
        "{:?}",
        built.logs
    );
    assert_eq!(
        built.logs.last().map(String::as_str),
        Some("Chat provider: Final request model=gpt-4o, input_items=1")
    );
}

#[test]
fn carries_the_client_and_sets_the_responses_call_type() {
    let built = build("{}");
    let projection = built.outcome.expect("the request builds");
    assert_eq!(projection.client, json!("marker"));
    assert_eq!(built.call_type, "responses");
}

#[test]
fn carries_a_null_client() {
    let mut state = logging_state();
    let mut optional_params = map("{}");
    let litellm_params = map("{}");
    let headers = string_map("{}");
    let history = chat_messages(USER);
    let request = build_request(
        ChatRequestInput {
            model: "gpt-4o",
            messages: &history,
            optional_params: &mut optional_params,
            litellm_params: &litellm_params,
            headers: &headers,
            reasoning_auto_summary: false,
        },
        &mut state,
        None::<&str>,
    )
    .expect("the request builds");
    assert!(request.client.is_none());
}

#[test]
fn promotes_extra_body_keys_and_drops_the_rest() {
    let built = build(
        r#"{"reasoning_effort": "low", "extra_body": {"reasoning_effort": "high", "vendor": {"x": 1}, "store": true}}"#,
    );
    let projection = built.outcome.expect("the request builds");
    assert_eq!(
        built.optional_params,
        json!({"reasoning_effort": "high", "store": true})
    );
    assert_eq!(
        options(&projection.params),
        json!({"reasoning": {"effort": "high"}, "store": true})
    );
}

#[rstest]
#[case::empty_object(r#"{"extra_body": {}}"#, json!({}), json!({}))]
#[case::null(r#"{"extra_body": null}"#, json!({}), json!({}))]
#[case::unsupported_only(
    r#"{"extra_body": {"max_tokens": 9, "vendor": 1}}"#,
    json!({}),
    json!({})
)]
#[case::text_promoted(
    r#"{"extra_body": {"text": {"format": {"type": "text"}}}}"#,
    json!({"text": {"format": {"type": "text"}}}),
    json!({"text": {"format": {"type": "text"}}})
)]
fn removes_extra_body_from_the_options(
    #[case] optional: &str,
    #[case] expected_options: Value,
    #[case] expected_params: Value,
) {
    let built = build(optional);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(built.optional_params, expected_options);
    assert_eq!(options(&projection.params), expected_params);
}

#[rstest]
#[case::metadata_merged(
    r#"{"api_key": "k", "temperature": 9, "metadata": {"a": 1, "same": 1}, "litellm_metadata": {"b": 2, "same": 3}}"#,
    json!({"a": 1, "same": 3, "b": 2})
)]
#[case::no_metadata(r#"{"api_key": "k"}"#, Value::Null)]
#[case::non_object_metadata(r#"{"metadata": "x"}"#, Value::Null)]
#[case::empty_litellm_metadata(r#"{"litellm_metadata": {}}"#, Value::Null)]
fn sanitizes_the_sdk_params(#[case] litellm: &str, #[case] metadata: Value) {
    let built = build_with(USER, "{}", litellm, "{}", false);
    let projection = built.outcome.expect("the request builds");
    let merged = projection
        .litellm_params
        .get("litellm_metadata")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(merged, metadata);
    assert!(projection.litellm_params.get("temperature").is_none());
    assert!(projection.litellm_params.get("metadata").is_none());
}

#[test]
fn lets_sdk_model_and_input_override_the_chat_request() {
    let built = build_with(
        USER,
        "{}",
        r#"{"model": "vendor/model", "input": [{"role": "user"}]}"#,
        "{}",
        false,
    );
    let projection = built.outcome.expect("the request builds");
    assert_eq!(projection.params["model"], json!("vendor/model"));
    assert_eq!(projection.params["input"], json!([{"role": "user"}]));
    assert!(projection.litellm_params.get("model").is_none());
    assert!(projection.litellm_params.get("input").is_none());
}

#[test]
fn flattens_function_tools_with_null_defaults() {
    let built = build(
        r#"{"tools": [
            {"type": "function", "function": {"name": "f", "description": "d", "parameters": {"type": "object"}, "strict": true}},
            {"type": "function", "function": {"name": "g"}}
        ]}"#,
    );
    let projection = built.outcome.expect("the request builds");
    assert_eq!(
        options(&projection.params)["tools"],
        json!([
            {
                "name": "f",
                "parameters": {"type": "object"},
                "strict": true,
                "type": "function",
                "description": "d"
            },
            {
                "name": "g",
                "parameters": null,
                "strict": null,
                "type": "function",
                "description": null
            }
        ])
    );
}

#[test]
fn flattens_custom_tool_grammars() {
    let built = build(
        r#"{"tools": [
            {"type": "custom", "custom": {"name": "g", "description": "d", "format": {"type": "grammar", "grammar": {"definition": "start: x", "syntax": "lark"}}}},
            {"type": "custom", "custom": {"format": {"type": "grammar", "definition": "start: y"}}},
            {"type": "custom", "custom": {"name": "h", "description": null}}
        ]}"#,
    );
    let projection = built.outcome.expect("the request builds");
    assert_eq!(
        options(&projection.params)["tools"],
        json!([
            {
                "type": "custom",
                "name": "g",
                "description": "d",
                "format": {"type": "grammar", "definition": "start: x", "syntax": "lark"}
            },
            {
                "type": "custom",
                "name": "",
                "format": {"type": "grammar", "definition": "start: y"}
            },
            {"type": "custom", "name": "h"}
        ])
    );
}

#[test]
fn passes_unknown_tools_through() {
    let built = build(
        r#"{"tools": [
            {"type": "web_search"},
            {"type": "vendor", "vendor": {"x": 1}},
            {"type": "function", "function": "x"}
        ]}"#,
    );
    let projection = built.outcome.expect("the request builds");
    assert_eq!(
        options(&projection.params)["tools"],
        json!([
            {"type": "web_search"},
            {"type": "vendor", "vendor": {"x": 1}},
            {"type": "function", "function": "x"}
        ])
    );
}

#[test]
fn appends_a_web_search_tool_to_existing_tools() {
    let built = build(
        r#"{"tools": [{"type": "web_search"}], "web_search_options": {"type": "vendor", "search_context_size": "low"}}"#,
    );
    let projection = built.outcome.expect("the request builds");
    assert_eq!(
        options(&projection.params)["tools"],
        json!([
            {"type": "web_search"},
            {"type": "vendor", "search_context_size": "low"}
        ])
    );
}

#[test]
fn lets_a_later_tools_option_replace_an_earlier_web_search_tool() {
    // LiteLLM maps options in order and the tools arm replaces the list, so this order loses the search tool
    let built = build(
        r#"{"web_search_options": {"search_context_size": "high"}, "tools": [{"type": "vendor"}]}"#,
    );
    let projection = built.outcome.expect("the request builds");
    assert_eq!(
        options(&projection.params)["tools"],
        json!([{"type": "vendor"}])
    );
}

#[rstest]
#[case::empty_options(r#"{"web_search_options": {}}"#)]
#[case::non_object_options(r#"{"web_search_options": "x"}"#)]
fn adds_a_bare_web_search_tool(#[case] optional: &str) {
    let built = build(optional);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(
        options(&projection.params)["tools"],
        json!([{"type": "web_search"}])
    );
}

#[test]
fn skips_null_web_search_options() {
    let built = build(r#"{"web_search_options": null}"#);
    let projection = built.outcome.expect("the request builds");
    assert!(options(&projection.params).get("tools").is_none());
}

#[test]
fn appends_a_web_search_tool_per_call() {
    let mut params = Map::new();
    add_web_search_tool(&mut params, &json!({})).expect("the tool appends");
    add_web_search_tool(&mut params, &json!({"search_context_size": "low"}))
        .expect("the tool appends");
    assert_eq!(
        params.get("tools"),
        Some(&json!([
            {"type": "web_search"},
            {"type": "web_search", "search_context_size": "low"}
        ]))
    );
}

#[rstest]
#[case::dict_unchanged(
    false,
    r#"{"reasoning_effort": {"effort": "low", "summary": "brief"}}"#,
    json!({"effort": "low", "summary": "brief"})
)]
#[case::dict_unchanged_with_auto_summary(
    true,
    r#"{"reasoning_effort": {"effort": "low", "summary": "brief"}}"#,
    json!({"effort": "low", "summary": "brief"})
)]
#[case::scalar_default(false, r#"{"reasoning_effort": "low"}"#, json!({"effort": "low"}))]
#[case::scalar_with_auto_summary(
    true,
    r#"{"reasoning_effort": "low"}"#,
    json!({"effort": "low", "summary": "detailed"})
)]
fn maps_reasoning_effort(
    #[case] auto_summary: bool,
    #[case] optional: &str,
    #[case] expected: Value,
) {
    let built = build_with(USER, optional, "{}", "{}", auto_summary);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(options(&projection.params)["reasoning"], expected);
}

#[rstest]
#[case::caller_headers(
    r#"{"X-Caller": "1"}"#,
    r#"{"extra_headers": {"X-Sdk": "2"}}"#,
    json!({"X-Caller": "1"})
)]
#[case::sdk_headers("{}", r#"{"extra_headers": {"X-Sdk": "2"}}"#, json!({"X-Sdk": "2"}))]
#[case::no_headers("{}", "{}", Value::Null)]
fn prefers_caller_extra_headers(
    #[case] headers: &str,
    #[case] litellm: &str,
    #[case] expected: Value,
) {
    let built = build_with(USER, "{}", litellm, headers, false);
    let projection = built.outcome.expect("the request builds");
    assert_eq!(projection.extra_headers, expected);
    assert!(projection.litellm_params.get("extra_headers").is_none());
}
