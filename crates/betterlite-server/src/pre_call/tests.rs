#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;

use async_trait::async_trait;
use rstest::rstest;
use serde_json::{Map, Value, json};
use strum::{ParseError, VariantArray as _};

use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::key::PreCallKey;
use crate::pre_call::ports::{RouterView, StreamOptionsCheck};
use crate::pre_call::rewrites::{
    CliOverrides, RequestModel, apply_aliases, apply_overrides, file_search_store_ids,
    request_model, resolve_call_id, stream_usage_updates, streamable_models, write_queue_time,
};
use crate::pre_call::route_type::RouteType;

struct TestKey {
    aliases: Option<Map<String, Value>>,
}

impl PreCallKey for TestKey {
    fn team_id(&self) -> Option<&str> {
        None
    }

    fn key_aliases(&self) -> Option<&Map<String, Value>> {
        self.aliases.as_ref()
    }

    fn key_router_settings(&self) -> Option<&Value> {
        None
    }
}

struct TestRouter {
    resolved: Vec<String>,
}

#[async_trait]
impl RouterView<TestKey> for TestRouter {
    fn resolved_models(&self, _model: &str, _team_id: Option<&str>) -> Vec<String> {
        self.resolved.clone()
    }

    fn arm_prefetch(&self, _model: &str, _data: &Map<String, Value>) {}

    async fn fallbacks(&self) -> Option<Vec<Value>> {
        None
    }

    async fn fallback_models(&self, _model: &str, _fallbacks: &[Value]) -> Option<Vec<String>> {
        None
    }

    async fn authorize_alias(&self, _target: &str, _key: &TestKey) -> Result<(), PreCallFailure> {
        Ok(())
    }
}

struct SupportedModels(Vec<&'static str>);

impl StreamOptionsCheck for SupportedModels {
    fn supports(&self, litellm_model: &str) -> bool {
        self.0.contains(&litellm_model)
    }
}

fn object(value: &Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[rstest]
#[case::completion("acompletion", RouteType::Completion)]
#[case::realtime("_arealtime", RouteType::Realtime)]
#[case::responses_websocket("_aresponses_websocket", RouteType::ResponsesWebsocket)]
#[case::not_async("call_mcp_tool", RouteType::CallMcpTool)]
#[case::apply_guardrail("apply_guardrail", RouteType::ApplyGuardrail)]
#[case::anthropic("anthropic_messages", RouteType::AnthropicMessages)]
#[case::image_generation("aimage_generation", RouteType::ImageGeneration)]
#[case::moderation("amoderation", RouteType::Moderation)]
#[case::rerank("arerank", RouteType::Rerank)]
#[case::speech("aspeech", RouteType::Speech)]
#[case::transcription("atranscription", RouteType::Transcription)]
#[case::realtime_transcription(
    "acreate_realtime_transcription_session",
    RouteType::CreateRealtimeTranscription
)]
fn route_type_round_trips(#[case] text: &str, #[case] expected: RouteType) {
    assert_eq!(text.parse::<RouteType>().unwrap(), expected);
    assert_eq!(<&str>::from(expected), text);
}

#[test]
fn every_route_type_text_parses_back_to_itself() {
    assert_eq!(RouteType::VARIANTS.len(), 97);
    assert!(
        RouteType::VARIANTS
            .iter()
            .all(|route| { <&str>::from(*route).parse::<RouteType>().as_ref() == Ok(route) })
    );
}

#[rstest]
#[case::unknown("acompletions")]
#[case::empty("")]
#[case::missing_underscore("arealtime")]
#[case::case_differs("Acompletion")]
fn route_type_rejects_unknown_text(#[case] text: &str) {
    assert_eq!(text.parse::<RouteType>(), Err(ParseError::VariantNotFound));
}

#[rstest]
#[case::absent(None, false)]
#[case::empty(Some(""), false)]
#[case::one_character(Some("a"), true)]
fn resolve_call_id_keeps_a_short_header(#[case] header: Option<&str>, #[case] kept: bool) {
    let resolved = resolve_call_id(header);
    assert_eq!(resolved == header.unwrap_or_default(), kept);
    if !kept {
        assert_eq!(resolved.len(), 36);
    }
}

#[rstest]
#[case::limit(256, true)]
#[case::over_limit(257, false)]
fn resolve_call_id_keeps_at_most_256_characters(#[case] length: usize, #[case] kept: bool) {
    let header = "x".repeat(length);
    let resolved = resolve_call_id(Some(&header));
    assert_eq!(resolved == header, kept);
    if !kept {
        assert_eq!(resolved.len(), 36);
    }
}

#[test]
fn resolve_call_id_counts_characters_not_bytes() {
    let header = "\u{e9}".repeat(256);
    assert_eq!(resolve_call_id(Some(&header)), header);
}

fn updates(data: &Value, always_include: Option<bool>, route: RouteType, supported: bool) -> Value {
    Value::Object(stream_usage_updates(
        &object(data),
        always_include,
        route,
        || supported,
    ))
}

#[rstest]
#[case::no_stream(json!({"model": "m"}), None, json!({}))]
#[case::no_stream_keeps_scrub(json!({"_litellm_strip_stream_usage": true}), None, json!({"_litellm_strip_stream_usage": false}))]
#[case::stream_one(json!({"stream": 1}), None, json!({}))]
#[case::stream_text(json!({"stream": "true"}), None, json!({}))]
#[case::stream_false(json!({"stream": false}), Some(true), json!({}))]
#[case::always_without_options(json!({"stream": true}), Some(true), json!({"stream_options": {"include_usage": true}}))]
#[case::always_adds_after_existing(json!({"stream": true, "stream_options": {"a": 1}}), Some(true), json!({"stream_options": {"a": 1, "include_usage": true}}))]
#[case::always_keeps_client_choice(json!({"stream": true, "stream_options": {"include_usage": false}}), Some(true), json!({}))]
#[case::always_ignores_non_object(json!({"stream": true, "stream_options": null}), Some(true), json!({}))]
#[case::always_with_scrub(json!({"stream": true, "_litellm_strip_stream_usage": true}), Some(true), json!({"_litellm_strip_stream_usage": false, "stream_options": {"include_usage": true}}))]
#[case::never(json!({"stream": true}), Some(false), json!({}))]
#[case::injected(json!({"stream": true}), None, json!({"stream_options": {"include_usage": true}, "_litellm_strip_stream_usage": true}))]
#[case::injected_keeps_options(json!({"stream": true, "stream_options": {"a": 1}}), None, json!({"stream_options": {"a": 1, "include_usage": true}, "_litellm_strip_stream_usage": true}))]
#[case::injected_over_false(json!({"stream": true, "stream_options": {"include_usage": false}}), None, json!({"stream_options": {"include_usage": true}, "_litellm_strip_stream_usage": true}))]
#[case::injected_replaces_scrub(json!({"stream": true, "_litellm_strip_stream_usage": false}), None, json!({"stream_options": {"include_usage": true}, "_litellm_strip_stream_usage": true}))]
#[case::client_asked(json!({"stream": true, "stream_options": {"include_usage": true}}), None, json!({}))]
fn chat_stream_usage_updates(
    #[case] data: Value,
    #[case] always_include: Option<bool>,
    #[case] expected: Value,
) {
    let found = updates(&data, always_include, RouteType::Completion, true);
    assert_eq!(found, expected);
}

#[test]
fn unsupported_models_get_no_injection() {
    let found = updates(&json!({"stream": true}), None, RouteType::Completion, false);
    assert_eq!(found, json!({}));
}

#[test]
fn other_routes_get_no_injection() {
    let found = updates(&json!({"stream": true}), None, RouteType::Responses, true);
    assert_eq!(found, json!({}));
}

#[test]
fn other_routes_still_take_the_always_include_merge() {
    let found = updates(
        &json!({"stream": true}),
        Some(true),
        RouteType::Embedding,
        false,
    );
    assert_eq!(found, json!({"stream_options": {"include_usage": true}}));
}

#[rstest]
#[case::not_a_stream(json!({}), None, RouteType::Completion)]
#[case::always_on(json!({"stream": true}), Some(true), RouteType::Completion)]
#[case::always_off(json!({"stream": true}), Some(false), RouteType::Completion)]
#[case::other_route(json!({"stream": true}), None, RouteType::Responses)]
#[case::client_asked(json!({"stream": true, "stream_options": {"include_usage": true}}), None, RouteType::Completion)]
fn the_support_check_runs_only_when_it_decides(
    #[case] data: Value,
    #[case] always_include: Option<bool>,
    #[case] route: RouteType,
) {
    let calls = Cell::new(0_u32);
    stream_usage_updates(&object(&data), always_include, route, || {
        calls.set(calls.get().saturating_add(1));
        true
    });
    assert_eq!(calls.get(), 0);
}

#[test]
fn the_support_check_runs_once_for_a_plain_chat_stream() {
    let calls = Cell::new(0_u32);
    stream_usage_updates(
        &object(&json!({"stream": true})),
        None,
        RouteType::Completion,
        || {
            calls.set(calls.get().saturating_add(1));
            true
        },
    );
    assert_eq!(calls.get(), 1);
}

#[rstest]
#[case::not_a_string(json!(7), vec!["gpt"], vec![], false)]
#[case::model_itself(json!("gpt"), vec!["gpt"], vec![], true)]
#[case::model_unsupported(json!("other"), vec!["gpt"], vec![], false)]
#[case::router_models_all_supported(json!("group"), vec!["a", "b"], vec!["a", "b"], true)]
#[case::router_model_unsupported(json!("group"), vec!["a"], vec!["a", "b"], false)]
#[case::router_models_replace_the_name(json!("gpt"), vec!["gpt"], vec!["b"], false)]
fn streamable_models_uses_the_resolved_models(
    #[case] model: Value,
    #[case] supported: Vec<&'static str>,
    #[case] resolved: Vec<&str>,
    #[case] expected: bool,
) {
    let router = TestRouter {
        resolved: resolved.into_iter().map(str::to_owned).collect(),
    };
    let check = SupportedModels(supported);
    assert_eq!(
        streamable_models(&model, Some(&router), None, &check),
        expected
    );
}

#[test]
fn streamable_models_without_a_router_checks_the_model() {
    let check = SupportedModels(vec!["gpt"]);
    assert!(streamable_models::<TestKey>(
        &json!("gpt"),
        None,
        None,
        &check
    ));
}

fn store_ids(data: &Value) -> Result<Vec<String>, PreCallFailure> {
    file_search_store_ids(&object(data)).map(|ids| ids.into_iter().collect())
}

fn bad_request() -> PreCallFailure {
    PreCallFailure::Http {
        status: 400,
        detail: json!({"error": "file_search.vector_store_ids must be a list of strings"}),
    }
}

#[test]
fn file_search_ids_are_sorted_and_deduplicated() {
    let data = json!({"tools": [
        {"type": "file_search", "vector_store_ids": ["b", "a"]},
        {"type": "function", "vector_store_ids": "ignored"},
        "not an object",
        {"type": "file_search", "vector_store_ids": ["a", "c"]},
    ]});
    assert_eq!(store_ids(&data).unwrap(), vec!["a", "b", "c"]);
}

#[rstest]
#[case::no_tools(json!({}))]
#[case::tools_not_a_list(json!({"tools": {"type": "file_search", "vector_store_ids": 5}}))]
#[case::missing(json!({"tools": [{"type": "file_search"}]}))]
#[case::null(json!({"tools": [{"type": "file_search", "vector_store_ids": null}]}))]
#[case::false_value(json!({"tools": [{"type": "file_search", "vector_store_ids": false}]}))]
#[case::zero(json!({"tools": [{"type": "file_search", "vector_store_ids": 0}]}))]
#[case::zero_float(json!({"tools": [{"type": "file_search", "vector_store_ids": 0.0}]}))]
#[case::empty_list(json!({"tools": [{"type": "file_search", "vector_store_ids": []}]}))]
fn file_search_ids_can_be_empty(#[case] data: Value) {
    assert!(store_ids(&data).unwrap().is_empty());
}

#[rstest]
#[case::string(json!("vs_1"))]
#[case::object(json!({"a": 1}))]
#[case::true_value(json!(true))]
#[case::non_zero_number(json!(3))]
#[case::empty_id(json!([""]))]
#[case::number_id(json!([1]))]
#[case::null_id(json!([null]))]
#[case::nested_list(json!([["vs_1"]]))]
fn file_search_ids_must_be_a_list_of_strings(#[case] ids: Value) {
    let data = json!({"tools": [{"type": "file_search", "vector_store_ids": ids}]});
    assert_eq!(store_ids(&data).unwrap_err(), bad_request());
}

#[test]
fn file_search_tools_after_a_bad_one_are_not_read() {
    let data = json!({"tools": [
        {"type": "file_search", "vector_store_ids": ["a", 1]},
        {"type": "file_search", "vector_store_ids": ["b"]},
    ]});
    assert_eq!(store_ids(&data).unwrap_err(), bad_request());
}

#[rstest]
#[case::string(json!({"model": "gpt"}), RequestModel::One("gpt".to_owned()))]
#[case::list(json!({"model": ["a", "b"]}), RequestModel::Many(vec!["a".to_owned(), "b".to_owned()]))]
#[case::empty_list(json!({"model": []}), RequestModel::Many(vec![]))]
#[case::mixed_list(json!({"model": ["a", 1]}), RequestModel::Absent)]
#[case::number(json!({"model": 3}), RequestModel::Absent)]
#[case::object(json!({"model": {"a": "b"}}), RequestModel::Absent)]
#[case::null(json!({"model": null}), RequestModel::Absent)]
#[case::missing(json!({}), RequestModel::Absent)]
fn request_model_keeps_a_string_or_a_string_list(
    #[case] data: Value,
    #[case] expected: RequestModel,
) {
    assert_eq!(request_model(&object(&data)), expected);
}

#[test]
fn overrides_write_truthy_values() {
    let mut data = Map::new();
    let overrides = CliOverrides {
        temperature: Some(0.5),
        request_timeout: Some(30.0),
        max_tokens: Some(64),
        api_base: Some("http://base".to_owned()),
    };
    apply_overrides(&mut data, &overrides);
    assert_eq!(
        Value::Object(data),
        json!({"temperature": 0.5, "request_timeout": 30.0, "max_tokens": 64, "api_base": "http://base"})
    );
}

#[test]
fn overrides_skip_zero_and_empty_values() {
    let mut data = object(&json!({"temperature": 0.9, "max_tokens": 10}));
    let overrides = CliOverrides {
        temperature: Some(0.0),
        request_timeout: Some(0.0),
        max_tokens: Some(0),
        api_base: Some(String::new()),
    };
    apply_overrides(&mut data, &overrides);
    assert_eq!(
        Value::Object(data),
        json!({"temperature": 0.9, "max_tokens": 10})
    );
}

#[test]
fn unset_overrides_change_nothing() {
    let mut data = object(&json!({"max_tokens": 10}));
    apply_overrides(&mut data, &CliOverrides::default());
    assert_eq!(Value::Object(data), json!({"max_tokens": 10}));
}

fn aliased(model: &Value, config: &[(&str, &str)], key_aliases: Option<&Value>) -> Value {
    let mut data = Map::new();
    data.insert("model".to_owned(), model.clone());
    let alias_map: BTreeMap<String, String> = config
        .iter()
        .map(|(alias, target)| ((*alias).to_owned(), (*target).to_owned()))
        .collect();
    let key = TestKey {
        aliases: key_aliases.map(object),
    };
    apply_aliases(&mut data, &alias_map, &key);
    data["model"].clone()
}

#[test]
fn config_alias_replaces_the_model() {
    assert_eq!(aliased(&json!("a"), &[("a", "b")], None), json!("b"));
}

#[test]
fn key_alias_replaces_the_model() {
    let key_aliases = json!({"a": "c"});
    assert_eq!(aliased(&json!("a"), &[], Some(&key_aliases)), json!("c"));
}

#[test]
fn key_alias_applies_to_the_config_result() {
    let key_aliases = json!({"b": "c"});
    assert_eq!(
        aliased(&json!("a"), &[("a", "b")], Some(&key_aliases)),
        json!("c")
    );
}

#[test]
fn key_alias_value_is_used_as_given() {
    let key_aliases = json!({"a": ["x", "y"]});
    assert_eq!(
        aliased(&json!("a"), &[], Some(&key_aliases)),
        json!(["x", "y"])
    );
}

#[test]
fn unmatched_and_non_string_models_stay() {
    let key_aliases = json!({"7": "c"});
    assert_eq!(
        aliased(&json!("z"), &[("a", "b")], Some(&key_aliases)),
        json!("z")
    );
    assert_eq!(
        aliased(&json!(7), &[("7", "b")], Some(&key_aliases)),
        json!(7)
    );
}

#[test]
fn queue_time_creates_the_bucket() {
    let mut data = object(&json!({"proxy_server_request": {"arrival_time": 100.0}}));
    write_queue_time(&mut data, "metadata", 102.5);
    assert_eq!(data["metadata"], json!({"queue_time_seconds": 2.5}));
}

#[test]
fn queue_time_keeps_the_bucket_fields() {
    let mut data = object(&json!({
        "proxy_server_request": {"arrival_time": 10},
        "litellm_metadata": {"a": 1},
    }));
    write_queue_time(&mut data, "litellm_metadata", 11.5);
    assert_eq!(
        data["litellm_metadata"],
        json!({"a": 1, "queue_time_seconds": 1.5})
    );
}

#[test]
fn queue_time_replaces_a_bucket_that_is_not_an_object() {
    let mut data = object(&json!({
        "proxy_server_request": {"arrival_time": 1.0},
        "metadata": "text",
    }));
    write_queue_time(&mut data, "metadata", 2.0);
    assert_eq!(data["metadata"], json!({"queue_time_seconds": 1.0}));
}

#[rstest]
#[case::no_request(json!({}))]
#[case::no_arrival(json!({"proxy_server_request": {}}))]
#[case::null_arrival(json!({"proxy_server_request": {"arrival_time": null}}))]
#[case::text_arrival(json!({"proxy_server_request": {"arrival_time": "1"}}))]
#[case::request_not_object(json!({"proxy_server_request": 5}))]
fn queue_time_needs_a_numeric_arrival(#[case] data: Value) {
    let mut fields = object(&data);
    write_queue_time(&mut fields, "metadata", 5.0);
    assert_eq!(Value::Object(fields), data);
}
