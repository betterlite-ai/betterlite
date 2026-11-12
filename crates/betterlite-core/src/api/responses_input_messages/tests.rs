#![cfg(test)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache, Ttl};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use serde_json::{Value, json};

use super::error::InputMessagesError;
use super::item::{is_function_call, is_tool_call_output};
use super::thinking::{ThinkingError, decode_thinking_blocks};
use super::{InputRequest, input_messages};
use crate::api::api_boundaries::ApiLimits;

const GREETING: &str = r#"Hosted web search: {"id":"w","action":{"type":"search","query":"q"},"status":"completed","type":"web_search_call"}"#;

fn json_input(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn cache(entries: &[(&str, &str)]) -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    let shared = DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    );
    for (key, value) in entries {
        let stored = serde_json::from_str::<CacheValue>(value).unwrap();
        shared.set(key, stored, Ttl::Absent);
    }
    shared
}

fn convert(
    input: &str,
    instructions: Option<&str>,
    replay: bool,
    cache: &DualLocalCache,
) -> Result<Vec<Value>, InputMessagesError> {
    let request = InputRequest {
        input: Some(json_input(input)),
        instructions: instructions.map(str::to_owned),
    };
    input_messages(&request, replay, ApiLimits::default(), cache)
}

fn messages(input: &str, replay: bool) -> Vec<Value> {
    convert(input, None, replay, &cache(&[])).unwrap()
}

fn cached_messages(input: &str, entries: &[(&str, &str)]) -> Vec<Value> {
    convert(input, None, false, &cache(entries)).unwrap()
}

fn limits_with_json(max_json_bytes: usize) -> ApiLimits {
    let default = ApiLimits::default();
    ApiLimits {
        max_json_bytes,
        max_event_bytes: default.max_event_bytes,
        max_items: default.max_items,
    }
}

#[rstest]
#[case::string_input(r#""hi""#, None, json!([{"role": "user", "content": "hi"}]))]
#[case::empty_string(r#""""#, None, json!([{"role": "user", "content": ""}]))]
#[case::instructions_first(
    r#""hi""#,
    Some("follow"),
    json!([{"role": "system", "content": "follow"}, {"role": "user", "content": "hi"}])
)]
#[case::empty_instructions(r#""hi""#, Some(""), json!([{"role": "user", "content": "hi"}]))]
#[case::whitespace_instructions(
    r#""hi""#,
    Some(" "),
    json!([{"role": "system", "content": " "}, {"role": "user", "content": "hi"}])
)]
#[case::null_input("null", None, json!([]))]
#[case::empty_list("[]", None, json!([]))]
fn message_inputs(
    #[case] input: &str,
    #[case] instructions: Option<&str>,
    #[case] expected: Value,
) {
    assert_eq!(
        Value::Array(convert(input, instructions, false, &cache(&[])).unwrap()),
        expected
    );
}

#[rstest]
#[case::skips_missing_content(r#"[{"role": "user"}]"#, json!([]))]
#[case::skips_null_content(r#"[{"role": "user", "content": null}]"#, json!([]))]
#[case::keeps_empty_content(
    r#"[{"role": "user", "content": ""}]"#,
    json!([{"role": "user", "content": ""}])
)]
#[case::defaults_missing_role(
    r#"[{"content": "x"}]"#,
    json!([{"role": "user", "content": "x"}])
)]
#[case::defaults_empty_role(
    r#"[{"role": "", "content": "x"}]"#,
    json!([{"role": "user", "content": "x"}])
)]
#[case::defaults_null_role(
    r#"[{"role": null, "content": "x"}]"#,
    json!([{"role": "user", "content": "x"}])
)]
#[case::keeps_assistant_role(
    r#"[{"role": "assistant", "content": "x"}]"#,
    json!([{"role": "assistant", "content": "x"}])
)]
#[case::forwards_unknown_role(
    r#"[{"role": "banana", "content": "x"}]"#,
    json!([{"role": "banana", "content": "x"}])
)]
#[case::unknown_type_uses_content(
    r#"[{"type": "mystery", "role": "system", "content": "x"}]"#,
    json!([{"role": "system", "content": "x"}])
)]
#[case::content_list_is_converted(
    r#"[{"role": "user", "content": [{"type": "input_text", "text": "a"}, {"type": "input_image", "image_url": "https://a.test/i", "detail": "low"}]}]"#,
    json!([{"role": "user", "content": [
        {"type": "text", "text": "a"},
        {"type": "image_url", "image_url": {"url": "https://a.test/i", "detail": "low"}}
    ]}])
)]
fn ordinary_items(#[case] input: &str, #[case] expected: Value) {
    assert_eq!(Value::Array(messages(input, false)), expected);
}

#[rstest]
#[case::number_role(r#"[{"role": 5, "content": "x"}]"#)]
#[case::list_role(r#"[{"role": ["user"], "content": "x"}]"#)]
fn rejects_non_text_roles(#[case] input: &str) {
    assert!(matches!(
        convert(input, None, false, &cache(&[])),
        Err(InputMessagesError::Role(_))
    ));
}

#[rstest]
#[case::bool_content(r#"[{"role": "user", "content": true}]"#)]
#[case::number_content(r#"[{"role": "user", "content": 3}]"#)]
#[case::object_content(r#"[{"role": "user", "content": {"text": "x"}}]"#)]
fn rejects_unconvertible_content(#[case] input: &str) {
    assert!(matches!(
        convert(input, None, false, &cache(&[])),
        Err(InputMessagesError::Content(_))
    ));
}

#[rstest]
#[case::null_item("[null]", 0)]
#[case::string_item(r#"["x"]"#, 0)]
#[case::number_item("[5]", 0)]
#[case::list_item("[[1]]", 0)]
#[case::second_item(r#"[{"role": "user", "content": "ok"}, 3]"#, 1)]
fn rejects_non_object_items(#[case] input: &str, #[case] position: usize) {
    let error = convert(input, None, false, &cache(&[])).unwrap_err();
    assert!(matches!(
        error,
        InputMessagesError::ItemNotObject { position: found } if found == position
    ));
}

#[rstest]
#[case::number_input("7")]
#[case::object_input(r#"{"input": "hi"}"#)]
#[case::bool_input("true")]
fn rejects_non_text_non_list_input(#[case] input: &str) {
    assert!(matches!(
        convert(input, None, false, &cache(&[])),
        Err(InputMessagesError::InputShape)
    ));
}

#[test]
fn web_search_calls_serialize_the_validated_model() {
    let input = concat!(
        r#"[{"type": "web_search_call", "id": "ws_1", "status": "completed","#,
        r#" "action": {"type": "search", "query": "q","#,
        r#" "sources": [{"type": "url", "url": "https://a.test"}]}, "note": "kept"}]"#
    );
    let text = concat!(
        r#"Hosted web search: {"id":"ws_1","action":{"type":"search","query":"q","#,
        r#""sources":[{"type":"url","url":"https://a.test"}]},"status":"completed","#,
        r#""type":"web_search_call","note":"kept"}"#
    );
    assert_eq!(
        messages(input, false),
        vec![json!({"role": "assistant", "content": text})]
    );
}

#[rstest]
#[case::open_page(
    r#"[{"type": "web_search_call", "id": "w", "status": "searching", "action": {"type": "open_page", "url": "https://b.test"}}]"#,
    r#"Hosted web search: {"id":"w","action":{"type":"open_page","url":"https://b.test"},"status":"searching","type":"web_search_call"}"#
)]
#[case::find_in_page(
    r#"[{"type": "web_search_call", "id": "w", "status": "failed", "action": {"type": "find_in_page", "pattern": "p", "url": "https://c.test"}}]"#,
    r#"Hosted web search: {"id":"w","action":{"pattern":"p","type":"find_in_page","url":"https://c.test"},"status":"failed","type":"web_search_call"}"#
)]
#[case::null_optionals_are_dropped(
    r#"[{"type": "web_search_call", "id": "w", "status": "in_progress", "action": {"type": "search", "queries": null, "query": "q", "sources": null}}]"#,
    r#"Hosted web search: {"id":"w","action":{"type":"search","query":"q"},"status":"in_progress","type":"web_search_call"}"#
)]
#[case::empty_lists_are_kept(
    r#"[{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "search", "queries": [], "sources": []}}]"#,
    r#"Hosted web search: {"id":"w","action":{"type":"search","queries":[],"sources":[]},"status":"completed","type":"web_search_call"}"#
)]
#[case::null_extras_are_dropped(
    r#"[{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "open_page", "url": "https://a.test", "note": null, "keep": false}, "gone": null}]"#,
    r#"Hosted web search: {"id":"w","action":{"type":"open_page","url":"https://a.test","keep":false},"status":"completed","type":"web_search_call"}"#
)]
fn web_search_shapes(#[case] input: &str, #[case] text: &str) {
    assert_eq!(
        messages(input, false),
        vec![json!({"role": "assistant", "content": text})]
    );
}

#[rstest]
#[case::missing_action(r#"{"type": "web_search_call", "id": "w", "status": "completed"}"#)]
#[case::action_not_object(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": "search"}"#
)]
#[case::missing_id(
    r#"{"type": "web_search_call", "status": "completed", "action": {"type": "search"}}"#
)]
#[case::number_id(
    r#"{"type": "web_search_call", "id": 1, "status": "completed", "action": {"type": "search"}}"#
)]
#[case::missing_status(r#"{"type": "web_search_call", "id": "w", "action": {"type": "search"}}"#)]
#[case::unknown_status(
    r#"{"type": "web_search_call", "id": "w", "status": "weird", "action": {"type": "search"}}"#
)]
#[case::unknown_action(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "nope"}}"#
)]
#[case::search_query_not_text(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "search", "query": 5}}"#
)]
#[case::search_queries_not_text_list(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "search", "queries": ["a", 1]}}"#
)]
#[case::sources_not_list(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "search", "sources": "x"}}"#
)]
#[case::source_not_object(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "search", "sources": [1]}}"#
)]
#[case::source_type_not_url(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "search", "sources": [{"type": "other", "url": "u"}]}}"#
)]
#[case::source_missing_url(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "search", "sources": [{"type": "url"}]}}"#
)]
#[case::find_missing_pattern(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "find_in_page", "url": "u"}}"#
)]
#[case::find_missing_url(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "find_in_page", "pattern": "p"}}"#
)]
#[case::open_page_url_not_text(
    r#"{"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "open_page", "url": 5}}"#
)]
fn web_search_rejections(#[case] item: &str) {
    let input = format!("[{item}]");
    assert!(matches!(
        convert(&input, None, false, &cache(&[])),
        Err(InputMessagesError::WebSearch(_))
    ));
}

#[test]
fn consecutive_calls_merge_into_one_assistant() {
    let input = concat!(
        r#"[{"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"},"#,
        r#" {"type": "custom_tool_call", "call_id": "c2", "name": "g", "input": "patch"}]"#
    );
    assert_eq!(
        messages(input, false),
        vec![json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0},
                {"id": "c2", "type": "function", "function": {"name": "g", "arguments": "{\"content\": \"patch\"}"}, "index": 0}
            ]
        })]
    );
}

#[test]
fn consecutive_calls_merge_into_a_trailing_assistant_with_text() {
    let input = concat!(
        r#"[{"role": "assistant", "content": "hello"},"#,
        r#" {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"}]"#
    );
    assert_eq!(
        messages(input, false),
        vec![json!({
            "role": "assistant",
            "content": "hello",
            "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
            ]
        })]
    );
}

#[test]
fn calls_after_a_user_message_stay_separate() {
    let input = concat!(
        r#"[{"role": "user", "content": "u"},"#,
        r#" {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"}]"#
    );
    assert_eq!(
        messages(input, false),
        vec![
            json!({"role": "user", "content": "u"}),
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
                ]
            }),
        ]
    );
}

#[test]
fn ordered_calls_correlate_cached_results() {
    let input = concat!(
        r#"[{"role": "assistant", "content": "hello"},"#,
        r#" {"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"},"#,
        r#" {"type": "custom_tool_call", "call_id": "c2", "name": "g", "input": "patch"},"#,
        r#" {"type": "web_search_call", "id": "w", "status": "completed", "action": {"type": "search", "query": "q"}},"#,
        r#" {"type": "function_call_output", "call_id": "c1", "output": "r1"},"#,
        r#" {"type": "custom_tool_call_output", "call_id": "c2", "output": "r2"}]"#
    );
    let cache = cache(&[
        (
            "c1",
            r#"{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}"#,
        ),
        (
            "c2",
            r#"{"id": "cX", "type": "function", "function": {"name": "g", "arguments": "{}"}}"#,
        ),
    ]);
    let expected = json!([
        {
            "role": "assistant",
            "content": [{"type": "text", "text": "hello"}, {"type": "text", "text": GREETING}],
            "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0},
                {"id": "c2", "type": "function", "function": {"name": "g", "arguments": "{\"content\": \"patch\"}"}, "index": 0}
            ]
        },
        {"role": "tool", "content": "r1", "tool_call_id": "c1"},
        {
            "role": "assistant",
            "tool_calls": [
                {"id": "cX", "type": "function", "function": {"name": "g", "arguments": "{}"}, "index": 0}
            ]
        },
        {"role": "tool", "content": "r2", "tool_call_id": "c2"}
    ]);
    assert_eq!(
        Value::Array(convert(input, None, false, &cache).unwrap()),
        expected
    );
}

#[test]
fn a_call_after_a_tool_result_opens_a_new_assistant() {
    let input = concat!(
        r#"[{"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"},"#,
        r#" {"type": "function_call_output", "call_id": "c1", "output": "r1"},"#,
        r#" {"type": "function_call", "call_id": "c3", "name": "h", "arguments": "{}"},"#,
        r#" {"type": "function_call_output", "call_id": "c3", "output": "r3"}]"#
    );
    let entries = [(
        "c3",
        r#"{"id": "", "type": "function", "function": {"name": "h", "arguments": "{}"}}"#,
    )];
    let expected = json!([
        {
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
            ]
        },
        {"role": "tool", "content": "r1", "tool_call_id": "c1"},
        {
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {"id": "c3", "type": "function", "function": {"name": "h", "arguments": "{}"}, "index": 0}
            ]
        },
        {
            "role": "assistant",
            "tool_calls": [
                {"id": "", "type": "function", "function": {"name": "h", "arguments": "{}"}, "index": 0}
            ]
        },
        {"role": "tool", "content": "r3", "tool_call_id": "c3"}
    ]);
    assert_eq!(
        Value::Array(convert(input, None, false, &cache(&entries)).unwrap()),
        expected
    );
}

#[test]
fn cached_wrapper_for_a_seen_call_is_dropped() {
    let input = concat!(
        r#"[{"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"},"#,
        r#" {"type": "function_call_output", "call_id": "c1", "output": "r1"}]"#
    );
    let entries = [(
        "c1",
        r#"{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}"#,
    )];
    assert_eq!(
        cached_messages(input, &entries),
        vec![
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
                ]
            }),
            json!({"role": "tool", "content": "r1", "tool_call_id": "c1"}),
        ]
    );
}

#[test]
fn cached_wrapper_with_a_new_id_is_kept() {
    let input = r#"[{"type": "function_call_output", "call_id": "c1", "output": "r1"}]"#;
    let entries = [(
        "c1",
        r#"{"id": "c9", "type": "function", "function": {"name": "f", "arguments": "{}"}}"#,
    )];
    assert_eq!(
        cached_messages(input, &entries),
        vec![
            json!({
                "role": "assistant",
                "tool_calls": [
                    {"id": "c9", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
                ]
            }),
            json!({"role": "tool", "content": "r1", "tool_call_id": "c1"}),
        ]
    );
}

#[test]
fn cached_wrapper_with_an_empty_id_is_kept() {
    let input = r#"[{"type": "function_call_output", "call_id": "c1", "output": "r1"}]"#;
    let entries = [(
        "c1",
        r#"{"type": "function", "function": {"name": "f", "arguments": "{}"}}"#,
    )];
    assert_eq!(
        cached_messages(input, &entries),
        vec![
            json!({
                "role": "assistant",
                "tool_calls": [
                    {"id": "", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
                ]
            }),
            json!({"role": "tool", "content": "r1", "tool_call_id": "c1"}),
        ]
    );
}

#[rstest]
#[case::empty(r#"{"type": "function_call_output", "call_id": "", "output": "r"}"#)]
#[case::zero(r#"{"type": "function_call_output", "call_id": 0, "output": "r"}"#)]
#[case::missing(r#"{"type": "function_call_output", "output": "r"}"#)]
fn empty_call_id_results_are_skipped(#[case] item: &str) {
    let input = format!("[{item}]");
    assert_eq!(messages(&input, false), Vec::<Value>::new());
}

#[rstest]
#[case::function(r#"{"type": "function_call_output", "call_id": "c", "output": "r"}"#, json!("r"))]
#[case::custom(r#"{"type": "custom_tool_call_output", "call_id": "c", "output": "r"}"#, json!("r"))]
#[case::computer(
    r#"{"type": "computer_call_output", "call_id": "c", "output": {"x": 1}}"#,
    json!("{\"x\": 1}")
)]
#[case::tool_result(
    r#"{"type": "tool_result", "call_id": "c", "output": [{"type": "input_text", "text": "a"}]}"#,
    json!("a")
)]
fn tool_output_types_become_tool_messages(#[case] item: &str, #[case] content: Value) {
    let input = format!("[{item}]");
    assert_eq!(
        messages(&input, false),
        vec![json!({"role": "tool", "content": content, "tool_call_id": "c"})]
    );
}

#[rstest]
#[case::function_call_output(Some("function_call_output"), true)]
#[case::custom_tool_call_output(Some("custom_tool_call_output"), true)]
#[case::computer_call_output(Some("computer_call_output"), true)]
#[case::tool_result(Some("tool_result"), true)]
#[case::function_call(Some("function_call"), false)]
#[case::message(Some("message"), false)]
#[case::empty(Some(""), false)]
#[case::missing(None, false)]
fn tool_output_predicate(#[case] kind: Option<&str>, #[case] expected: bool) {
    assert_eq!(is_tool_call_output(kind), expected);
}

#[rstest]
#[case::function_call(Some("function_call"), true)]
#[case::custom_tool_call(Some("custom_tool_call"), true)]
#[case::function_call_output(Some("function_call_output"), false)]
#[case::reasoning(Some("reasoning"), false)]
#[case::empty(Some(""), false)]
#[case::missing(None, false)]
fn function_call_predicate(#[case] kind: Option<&str>, #[case] expected: bool) {
    assert_eq!(is_function_call(kind), expected);
}

#[rstest]
#[case::text_content(
    r#"[{"type": "reasoning", "content": " x "}]"#,
    json!([{"role": "user", "content": " x "}])
)]
#[case::text_blocks_win(
    r#"[{"type": "reasoning", "content": [{"type": "output_text", "text": " a "}, {"type": "summary_text", "text": "b"}], "summary": [{"text": "s"}]}]"#,
    json!([{"role": "user", "content": [{"type": "text", "text": " a "}, {"type": "text", "text": "b"}]}])
)]
#[case::opaque_blocks_fall_back_to_summary(
    r#"[{"type": "reasoning", "content": [{"type": "encrypted_content", "encrypted_content": "opaque"}], "summary": [{"type": "summary_text", "text": " s "}, {"type": "other", "text": "t"}]}]"#,
    json!([{"role": "user", "content": "s\nt"}])
)]
#[case::opaque_blocks_without_summary_are_visible(
    r#"[{"type": "reasoning", "content": [{"type": "encrypted_content", "encrypted_content": "opaque"}]}]"#,
    json!([{"role": "user", "content": [{"type": "text", "text": "opaque"}]}])
)]
#[case::nothing_to_inspect(r#"[{"type": "reasoning"}]"#, json!([]))]
#[case::empty_content_stays(r#"[{"type": "reasoning", "content": "", "summary": []}]"#, json!([{"role": "user", "content": ""}]))]
#[case::unsupported_content_types_are_dropped(
    r#"[{"type": "reasoning", "content": [{"type": "text", "text": 5}, {"type": "text", "text": "  "}, "raw", 7]}]"#,
    json!([{"role": "user", "content": [{"type": "text", "text": 5}, {"type": "text", "text": "  "}, "raw"]}])
)]
#[case::role_is_kept(
    r#"[{"type": "reasoning", "role": "assistant", "content": "why"}]"#,
    json!([{"role": "assistant", "content": "why"}])
)]
#[case::summary_text_is_stripped_and_joined(
    r#"[{"type": "reasoning", "summary": [{"text": " a "}, {"text": "b"}, {"text": ""}]}]"#,
    json!([{"role": "user", "content": "a\nb"}])
)]
#[case::non_list_summary_is_ignored(
    r#"[{"type": "reasoning", "summary": "s"}]"#,
    json!([])
)]
fn reasoning_inspection(#[case] input: &str, #[case] expected: Value) {
    assert_eq!(Value::Array(messages(input, false)), expected);
}

#[rstest]
#[case::text_content(
    r#"[{"type": "reasoning", "content": " x "}]"#,
    json!([{"role": "assistant", "content": null, "reasoning_content": " x "}])
)]
#[case::text_blocks_strip_join_and_skip_opaque(
    r#"[{"type": "reasoning", "content": [{"type": "text", "text": " a "}, {"type": "encrypted_content", "text": "ignored"}, {"type": "redacted_thinking", "text": "ignored"}, {"type": "text", "text": "b"}]}]"#,
    json!([{"role": "assistant", "content": null, "reasoning_content": "a\nb"}])
)]
#[case::summary_is_the_fallback(
    r#"[{"type": "reasoning", "content": "", "summary": [{"text": " s "}, {"text": "t"}]}]"#,
    json!([{"role": "assistant", "content": null, "reasoning_content": "s\nt"}])
)]
#[case::signed_blocks_alone(
    r#"[{"type": "reasoning", "encrypted_content": "[{\"type\":\"redacted_thinking\",\"data\":\"d\"}]"}]"#,
    json!([{"role": "assistant", "content": null, "thinking_blocks": [{"type": "redacted_thinking", "data": "d"}]}])
)]
#[case::text_and_signed_blocks(
    r#"[{"type": "reasoning", "content": " x ", "encrypted_content": "[{\"type\":\"thinking\",\"signature\":\"sig\"}]"}]"#,
    json!([{"role": "assistant", "content": null, "reasoning_content": " x ", "thinking_blocks": [{"type": "thinking", "signature": "sig"}]}])
)]
#[case::empty_reasoning(r#"[{"type": "reasoning"}]"#, json!([]))]
#[case::non_text_encrypted_content(
    r#"[{"type": "reasoning", "encrypted_content": 5}]"#,
    json!([])
)]
#[case::blank_encrypted_content(
    r#"[{"type": "reasoning", "encrypted_content": "   "}]"#,
    json!([])
)]
fn reasoning_replay(#[case] input: &str, #[case] expected: Value) {
    assert_eq!(Value::Array(messages(input, true)), expected);
}

#[test]
fn reasoning_rejects_a_non_text_role() {
    assert!(matches!(
        convert(
            r#"[{"type": "reasoning", "role": 5, "content": "x"}]"#,
            None,
            false,
            &cache(&[])
        ),
        Err(InputMessagesError::Role(_))
    ));
}

#[test]
fn an_assistant_message_after_a_tool_result_is_not_merged() {
    let input = concat!(
        r#"[{"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"},"#,
        r#" {"type": "function_call_output", "call_id": "c1", "output": "r1"},"#,
        r#" {"role": "assistant", "content": "next"}]"#
    );
    assert_eq!(
        messages(input, false),
        vec![
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
                ]
            }),
            json!({"role": "tool", "content": "r1", "tool_call_id": "c1"}),
            json!({"role": "assistant", "content": "next"}),
        ]
    );
}

#[test]
fn pending_reasoning_merges_into_the_following_assistant() {
    let input = concat!(
        r#"[{"type": "reasoning", "content": " x "}, "#,
        r#"{"type": "reasoning", "content": "more"}, "#,
        r#"{"type": "message", "role": "assistant", "content": "answer"}]"#
    );
    assert_eq!(
        messages(input, true),
        vec![json!({
            "role": "assistant",
            "content": "answer",
            "reasoning_content": " x \nmore"
        })]
    );
}

#[test]
fn pending_reasoning_merges_into_a_tool_call_assistant() {
    let input = concat!(
        r#"[{"type": "reasoning", "content": "why"}, "#,
        r#"{"type": "function_call", "call_id": "c1", "name": "f", "arguments": "{}"}]"#
    );
    assert_eq!(
        messages(input, true),
        vec![json!({
            "role": "assistant",
            "content": null,
            "reasoning_content": "why",
            "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}, "index": 0}
            ]
        })]
    );
}

#[test]
fn pending_reasoning_before_a_user_message_stays_standalone() {
    let input = concat!(
        r#"[{"type": "reasoning", "content": "why"}, "#,
        r#"{"role": "user", "content": "u"}]"#
    );
    assert_eq!(
        messages(input, true),
        vec![
            json!({"role": "assistant", "content": null, "reasoning_content": "why"}),
            json!({"role": "user", "content": "u"}),
        ]
    );
}

#[test]
fn pending_reasoning_at_the_end_stays_standalone() {
    let input = concat!(
        r#"[{"type": "reasoning", "content": "a"}, "#,
        r#"{"type": "reasoning", "content": "b"}]"#
    );
    assert_eq!(
        messages(input, true),
        vec![
            json!({"role": "assistant", "content": null, "reasoning_content": "a"}),
            json!({"role": "assistant", "content": null, "reasoning_content": "b"}),
        ]
    );
}

#[rstest]
#[case::signed_blocks(
    br#"[{"type":"thinking","signature":"sig","thinking":"x","extra":{"k":1}},{"type":"redacted_thinking","data":"opaque"},{"type":"thinking","signature":""}]"#,
    2
)]
#[case::numeric_signals(
    br#"[{"type":"thinking","signature":1},{"type":"redacted_thinking","data":2}]"#,
    2
)]
fn decode_keeps_replayable_blocks(#[case] bytes: &[u8], #[case] count: usize) {
    let decoded = decode_thinking_blocks(bytes, ApiLimits::default()).unwrap();
    let blocks = decoded.unwrap();
    assert_eq!(blocks.len(), count);
}

#[test]
fn decode_preserves_blocks_in_order() {
    let bytes = concat!(
        r#"[{"type":"thinking","signature":"sig","thinking":"x","extra":{"k":1}},"#,
        r#"{"type":"redacted_thinking","data":"opaque"},"#,
        r#"{"type":"thinking","signature":""}]"#
    );
    assert_eq!(
        decode_thinking_blocks(bytes.as_bytes(), ApiLimits::default()).unwrap(),
        Some(vec![
            json!({"type": "thinking", "signature": "sig", "thinking": "x", "extra": {"k": 1}}),
            json!({"type": "redacted_thinking", "data": "opaque"}),
        ])
    );
}

#[rstest]
#[case::falsy_signals(br#"[{"type":"thinking","signature":0},{"type":"redacted_thinking","data":[]},{"type":"thinking","signature":{}}]"#)]
#[case::invalid_json(b"not json")]
#[case::json_object(br#"{"type":"thinking","signature":"s"}"#)]
#[case::empty_list(b"[]")]
#[case::no_replayable_blocks(br#"[{"type":"thinking"},{"type":"other","signature":"s"},1]"#)]
#[case::not_utf8(b"\xff\xfe")]
#[case::empty(b"")]
fn decode_returns_none(#[case] bytes: &[u8]) {
    assert_eq!(
        decode_thinking_blocks(bytes, ApiLimits::default()).unwrap(),
        None
    );
}

#[test]
fn decode_rejects_serialized_blocks_over_the_limit() {
    let bytes = b"[{\"type\":1}]";
    assert_eq!(
        decode_thinking_blocks(bytes, limits_with_json(8)),
        Err(ThinkingError::TooLarge {
            observed: bytes.len(),
            limit: 8,
        })
    );
}

#[test]
fn decode_accepts_serialized_blocks_at_the_limit() {
    let bytes = br#"[{"type":"thinking","signature":"s"}]"#;
    assert!(matches!(
        decode_thinking_blocks(bytes, limits_with_json(bytes.len())),
        Ok(Some(blocks)) if blocks.len() == 1
    ));
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/responses_input_messages");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (
                path.file_name().unwrap().to_owned(),
                decode_thinking_blocks(&bytes, ApiLimits::default()),
            )
        })
        .collect::<Vec<_>>();
    assert!(replayed.len() >= 9);
    assert!(replayed.iter().any(|(name, result)| name == "signed-blocks"
        && matches!(result, Ok(Some(blocks)) if blocks.len() == 2)));
    assert!(
        replayed
            .iter()
            .any(|(name, result)| name == "numeric-signals"
                && matches!(result, Ok(Some(blocks)) if blocks.len() == 2))
    );
    assert!(
        replayed
            .iter()
            .any(|(name, result)| name == "invalid-json" && matches!(result, Ok(None)))
    );
    assert!(
        replayed
            .iter()
            .any(|(name, result)| name == "falsy-signals" && matches!(result, Ok(None)))
    );
}
