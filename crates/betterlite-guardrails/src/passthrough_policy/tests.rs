#![cfg(test)]

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use betterlite_core::api::request_guardrail_policy::GuardrailMetadata;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::{
    JsonPathValue, PassthroughGuardrailSettings, collect_guardrails, evaluate, extract_fields,
    get_settings, normalize_config,
};

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn single(value: &str) -> JsonPathValue {
    JsonPathValue::Single(value.to_owned())
}

fn many(values: &[&str]) -> JsonPathValue {
    let texts = values.iter().map(|value| (*value).to_owned()).collect();
    JsonPathValue::Many(texts)
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn settings(request: Option<&[&str]>, response: Option<&[&str]>) -> PassthroughGuardrailSettings {
    PassthroughGuardrailSettings {
        request_fields: request.map(names),
        response_fields: response.map(names),
    }
}

fn metadata(guardrails: &[&str]) -> GuardrailMetadata {
    GuardrailMetadata {
        guardrails: names(guardrails),
        policies: Vec::new(),
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let output = with_default(subscriber, run);
    let logs = String::from_utf8(captured.0.lock().expect("poisoned").clone())
        .expect("tracing output is utf8");
    (output, logs)
}

#[test]
fn evaluate_reads_a_simple_key() {
    let data = json(r#"{"query": "hello"}"#);
    assert_eq!(evaluate(&data, "query"), Some(single("hello")));
}

#[test]
fn evaluate_reads_a_nested_key() {
    let data = json(r#"{"foo": {"bar": {"baz": "deep"}}}"#);
    assert_eq!(evaluate(&data, "foo.bar.baz"), Some(single("deep")));
}

#[test]
fn evaluate_collects_every_wildcard_match() {
    let data = json(r#"{"documents": [{"text": "first"}, {"text": "second"}]}"#);
    assert_eq!(
        evaluate(&data, "documents[*].text"),
        Some(many(&["first", "second"]))
    );
}

#[test]
fn evaluate_flattens_a_list_result_from_each_wildcard_item() {
    let data = json(r#"{"messages": [{"content": ["a", "b"]}, {"content": ["c"]}]}"#);
    assert_eq!(
        evaluate(&data, "messages[*].content"),
        Some(many(&["a", "b", "c"]))
    );
}

#[test]
fn evaluate_returns_none_when_no_wildcard_item_matches() {
    let data = json(r#"{"items": [{"name": "a"}]}"#);
    assert_eq!(evaluate(&data, "items[*].missing"), None);
}

#[test]
fn evaluate_skips_wildcard_items_that_are_not_objects() {
    let data = json(r#"{"items": [{"name": "a"}, "text", {"name": "b"}]}"#);
    assert_eq!(evaluate(&data, "items[*].name"), Some(many(&["a", "b"])));
}

#[test]
fn evaluate_returns_a_wildcard_list_without_a_remaining_path() {
    let data = json(r#"{"documents": ["a", "b"]}"#);
    assert_eq!(evaluate(&data, "documents[*]"), Some(many(&["a", "b"])));
}

#[test]
fn evaluate_returns_an_empty_list_for_an_empty_wildcard() {
    let data = json(r#"{"documents": []}"#);
    assert_eq!(evaluate(&data, "documents[*]"), Some(many(&[])));
}

#[rstest]
#[case::missing_key(r#"{"query": "hello"}"#, "absent")]
#[case::key_below_a_string(r#"{"foo": "text"}"#, "foo.bar")]
#[case::wildcard_on_a_string(r#"{"items": "text"}"#, "items[*].name")]
#[case::key_below_a_null(r#"{"foo": null}"#, "foo.bar")]
#[case::wildcard_on_an_object(r#"{"items": {"name": "a"}}"#, "items[*].name")]
fn evaluate_returns_none_for_missing_keys_and_type_mismatches(
    #[case] source: &str,
    #[case] expression: &str,
) {
    assert_eq!(evaluate(&json(source), expression), None);
}

#[test]
fn evaluate_returns_none_for_an_empty_expression() {
    assert_eq!(evaluate(&json(r#"{"query": "hello"}"#), ""), None);
}

#[test]
fn evaluate_returns_none_for_an_empty_object() {
    assert_eq!(evaluate(&json("{}"), "query"), None);
}

// LiteLLM re-splits the remaining path on the next wildcard, so a second wildcard sees an empty key
#[test]
fn evaluate_returns_none_for_a_nested_wildcard_path() {
    let data = json(r#"{"a": [{"b": [{"c": "x"}]}]}"#);
    assert_eq!(evaluate(&data, "a[*].b[*].c"), None);
}

#[test]
fn evaluate_renders_other_leaves_as_json_text() {
    let data = json(r#"{"count": 2, "enabled": true, "meta": {"a": 1}}"#);
    assert_eq!(evaluate(&data, "count"), Some(single("2")));
    assert_eq!(evaluate(&data, "enabled"), Some(single("true")));
    assert_eq!(evaluate(&data, "meta"), Some(single(r#"{"a":1}"#)));
}

#[test]
fn evaluate_renders_every_scalar_list_item() {
    let data = json(r#"{"items": ["a", 2, null, {"b": 1}]}"#);
    assert_eq!(
        evaluate(&data, "items[*]"),
        Some(many(&["a", "2", r#"{"b":1}"#]))
    );
}

#[test]
fn extract_fields_joins_values_with_newlines() {
    let data = json(r#"{"query": "hello", "documents": [{"text": "a"}, {"text": "b"}]}"#);
    let expressions = names(&["query", "documents[*].text"]);
    assert_eq!(extract_fields(&data, &expressions), "hello\na\nb");
}

#[test]
fn extract_fields_skips_an_expression_that_fails() {
    let data = json(r#"{"query": "hello"}"#);
    let expressions = names(&["absent", "query"]);
    assert_eq!(extract_fields(&data, &expressions), "hello");
}

#[test]
fn extract_fields_returns_an_empty_string_without_matches() {
    let data = json(r#"{"query": "hello"}"#);
    assert_eq!(extract_fields(&data, &names(&["absent"])), "");
    assert_eq!(extract_fields(&data, &[]), "");
}

#[rstest]
#[case::empty_single_value(r#"{"a": "", "b": "x"}"#, &["a", "b"], "x")]
#[case::empty_list_value(r#"{"a": []}"#, &["a"], "")]
#[case::list_with_an_empty_item(r#"{"a": ["", "x"]}"#, &["a"], "x")]
fn extract_fields_leaves_empty_values_out(
    #[case] source: &str,
    #[case] fields: &[&str],
    #[case] expected: &str,
) {
    assert_eq!(extract_fields(&json(source), &names(fields)), expected);
}

#[test]
fn extract_fields_logs_a_debug_line_for_a_failed_expression() {
    let data = json(r#"{"query": "hello"}"#);
    let (output, logs) = capture(|| extract_fields(&data, &names(&["absent"])));
    assert_eq!(output, "");
    assert!(logs.contains("DEBUG"));
    assert!(logs.contains("absent"));
}

#[test]
fn normalize_config_turns_name_list_into_settings_map() {
    let config = json(r#"["g1", "g2"]"#);
    let expected = IndexMap::from([("g1".to_owned(), None), ("g2".to_owned(), None)]);
    assert_eq!(normalize_config(Some(&config)), Some(expected));
}

#[test]
fn normalize_config_keeps_dict_settings() {
    let config = json(r#"{"g1": {"request_fields": ["query"]}, "g2": null}"#);
    let expected = IndexMap::from([
        (
            "g1".to_owned(),
            Some(json(r#"{"request_fields": ["query"]}"#)),
        ),
        ("g2".to_owned(), None),
    ]);
    assert_eq!(normalize_config(Some(&config)), Some(expected));
}

#[test]
fn normalize_config_returns_an_empty_map_for_an_empty_list() {
    assert_eq!(normalize_config(Some(&json("[]"))), Some(IndexMap::new()));
}

#[rstest]
#[case::no_config(None)]
#[case::null(Some("null"))]
fn normalize_config_returns_none_without_a_config(#[case] source: Option<&str>) {
    let config = source.map(json);
    assert_eq!(normalize_config(config.as_ref()), None);
}

#[rstest]
#[case::text(r#""g1""#)]
#[case::number("42")]
#[case::flag("true")]
fn normalize_config_returns_none_for_another_type(#[case] source: &str) {
    assert_eq!(normalize_config(Some(&json(source))), None);
}

#[test]
fn normalize_config_returns_none_for_a_non_string_name() {
    assert_eq!(normalize_config(Some(&json(r#"["g1", 2]"#))), None);
}

#[test]
fn normalize_config_keeps_an_unknown_settings_key() {
    let config = json(r#"{"g1": {"request_fields": ["query"], "extra": 1}}"#);
    let expected = IndexMap::from([(
        "g1".to_owned(),
        Some(json(r#"{"request_fields": ["query"], "extra": 1}"#)),
    )]);
    assert_eq!(normalize_config(Some(&config)), Some(expected));
}

#[test]
fn normalize_config_keeps_a_wrong_typed_settings_value() {
    let config = json(r#"{"g1": {"request_fields": "query"}}"#);
    let expected = IndexMap::from([(
        "g1".to_owned(),
        Some(json(r#"{"request_fields": "query"}"#)),
    )]);
    assert_eq!(normalize_config(Some(&config)), Some(expected));
}

#[test]
fn normalize_config_logs_a_debug_line_for_another_type() {
    let config = json("42");
    let (normalized, logs) = capture(|| normalize_config(Some(&config)));
    assert_eq!(normalized, None);
    assert!(logs.contains("DEBUG"));
    assert!(logs.contains("not a list or an object"));
}

#[test]
fn get_settings_reads_one_guardrails_settings() {
    let config = json(r#"{"g1": {"request_fields": ["query"], "response_fields": ["output"]}}"#);
    let expected = settings(Some(&["query"]), Some(&["output"]));
    assert_eq!(get_settings(Some(&config), "g1"), Some(expected));
}

#[test]
fn get_settings_ignores_an_unknown_settings_key() {
    let config = json(r#"{"g1": {"request_fields": ["query"], "extra": 1}}"#);
    assert_eq!(
        get_settings(Some(&config), "g1"),
        Some(settings(Some(&["query"]), None))
    );
}

#[test]
fn get_settings_returns_none_for_a_wrong_typed_settings_value() {
    let config = json(r#"{"g1": {"request_fields": "query"}}"#);
    assert_eq!(get_settings(Some(&config), "g1"), None);
}

#[rstest]
#[case::bare_name(r#"["g1"]"#, "g1")]
#[case::unknown_name(r#"{"g1": {"request_fields": ["query"]}}"#, "g2")]
#[case::no_config("null", "g1")]
fn get_settings_returns_none_without_settings(#[case] source: &str, #[case] name: &str) {
    assert_eq!(get_settings(Some(&json(source)), name), None);
}

#[test]
fn collect_guardrails_returns_passthrough_names() {
    let config = json(r#"["g1", "g2"]"#);
    let expected = HashMap::from([("g1".to_owned(), true), ("g2".to_owned(), true)]);
    assert_eq!(
        collect_guardrails(&metadata(&[]), &metadata(&[]), Some(&config)),
        Some(expected)
    );
}

#[test]
fn collect_guardrails_reads_a_settings_dict() {
    let config = json(r#"{"g1": {"request_fields": ["query"]}}"#);
    let expected = HashMap::from([("g1".to_owned(), true)]);
    assert_eq!(
        collect_guardrails(&metadata(&[]), &metadata(&[]), Some(&config)),
        Some(expected)
    );
}

#[test]
fn collect_guardrails_keeps_a_name_with_a_malformed_settings_object() {
    let config = json(r#"{"g1": {"request_fields": ["query"], "extra": 1}}"#);
    let expected = HashMap::from([("g1".to_owned(), true), ("key-a".to_owned(), true)]);
    assert_eq!(
        collect_guardrails(&metadata(&["key-a"]), &metadata(&[]), Some(&config)),
        Some(expected)
    );
}

#[test]
fn collect_guardrails_keeps_a_name_with_a_wrong_typed_settings_object() {
    let config = json(r#"{"g1": {"request_fields": "query"}}"#);
    let expected = HashMap::from([("g1".to_owned(), true), ("team-a".to_owned(), true)]);
    assert_eq!(
        collect_guardrails(&metadata(&[]), &metadata(&["team-a"]), Some(&config)),
        Some(expected)
    );
}

#[test]
fn collect_guardrails_adds_key_and_team_guardrails() {
    let config = json(r#"["g1"]"#);
    let expected = HashMap::from([
        ("g1".to_owned(), true),
        ("key-a".to_owned(), true),
        ("team-a".to_owned(), true),
    ]);
    assert_eq!(
        collect_guardrails(&metadata(&["key-a"]), &metadata(&["team-a"]), Some(&config)),
        Some(expected)
    );
}

#[test]
fn collect_guardrails_keeps_one_entry_per_name() {
    let config = json(r#"["shared"]"#);
    let expected = HashMap::from([
        ("shared".to_owned(), true),
        ("key-a".to_owned(), true),
        ("team-a".to_owned(), true),
    ]);
    let key = metadata(&["shared", "key-a"]);
    let team = metadata(&["key-a", "shared", "team-a"]);
    assert_eq!(
        collect_guardrails(&key, &team, Some(&config)),
        Some(expected)
    );
}

#[rstest]
#[case::no_config("null")]
#[case::empty_list("[]")]
#[case::empty_map("{}")]
fn collect_guardrails_returns_none_without_configured_guardrails(#[case] source: &str) {
    let key = metadata(&["key-a"]);
    assert_eq!(
        collect_guardrails(&key, &metadata(&[]), Some(&json(source))),
        None
    );
}

#[test]
fn collect_guardrails_returns_none_for_an_invalid_config() {
    assert_eq!(
        collect_guardrails(&metadata(&[]), &metadata(&[]), Some(&json("42"))),
        None
    );
}

#[test]
fn collect_guardrails_ignores_metadata_policies() {
    let config = json(r#"["g1"]"#);
    let key = GuardrailMetadata {
        guardrails: Vec::new(),
        policies: names(&["p1"]),
    };
    let expected = HashMap::from([("g1".to_owned(), true)]);
    assert_eq!(
        collect_guardrails(&key, &metadata(&[]), Some(&config)),
        Some(expected)
    );
}
