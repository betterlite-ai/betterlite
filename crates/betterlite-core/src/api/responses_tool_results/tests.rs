#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache, Ttl};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rstest::rstest;
use serde::Serialize;

use super::error::ToolResultsError;
use super::input_call::input_function_call;
use super::outbound_call::{
    ApplyPatchOperation, CallIdentity, FunctionToolCall, apply_patch_tool_call,
};
use super::records::{CachedCallMessage, FunctionChunk, ResultMessage, ToolCallChunk, ToolMessage};
use super::tool_output::{tool_call_output_messages, tool_output_content};
use crate::api::responses_codex_tools::records::AttributeValue;

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn fields(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn wire<Item>(value: &Item) -> Json
where
    Item: Serialize,
{
    parse(&serde_json::to_string(value).unwrap())
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

fn messages(item: &str, entries: &[(&str, &str)]) -> Vec<ResultMessage> {
    tool_call_output_messages(&fields(item), &cache(entries)).unwrap()
}

fn call_chunk(id: &str, kind: &str, name: &str, arguments: &str) -> ToolCallChunk {
    ToolCallChunk {
        id: id.to_owned(),
        kind: kind.to_owned(),
        function: FunctionChunk {
            name: name.to_owned(),
            arguments: arguments.to_owned(),
            provider_specific_fields: None,
        },
        index: 0,
        provider_specific_fields: None,
    }
}

fn tool_message(tool_call_id: &str, content: &str) -> ResultMessage {
    ResultMessage::Tool(ToolMessage {
        role: "tool",
        content: tool_output_content(Some(&Json::Text(content.to_owned()))),
        tool_call_id: tool_call_id.to_owned(),
    })
}

#[rstest]
#[case::absent(None, r#""""#)]
#[case::null(Some("null"), r#""""#)]
#[case::string_unchanged(Some(r#"" padded ""#), r#"" padded ""#)]
#[case::empty_string(Some(r#""""#), r#""""#)]
#[case::texts_join_without_separator(
    Some(
        r#"[{"type":"input_text","text":"a"},{"type":"output_text","text":"b"},{"type":"text","text":"c"}]"#
    ),
    r#""abc""#
)]
#[case::empty_and_non_string_text_ignored(
    Some(
        r#"[{"type":"text","text":""},{"type":"text","text":7},{"type":"text"},{"type":"text","text":"kept"}]"#
    ),
    r#""kept""#
)]
#[case::unknown_types_and_non_objects_ignored(
    Some(
        r#"["x",7,null,{"type":"other","text":"no"},{"text":"untyped"},{"type":7,"text":"no"},{"type":"text","text":"yes"}]"#
    ),
    r#""yes""#
)]
#[case::image_url_string(
    Some(r#"[{"type":"input_image","image_url":"https://x/a.png"}]"#),
    r#"[{"type":"image_url","image_url":{"url":"https://x/a.png"}}]"#
)]
#[case::image_url_object_drops_detail(
    Some(
        r#"[{"type":"image_url","image_url":{"url":"https://x/b.png","detail":"high"},"detail":"low"}]"#
    ),
    r#"[{"type":"image_url","image_url":{"url":"https://x/b.png"}}]"#
)]
#[case::url_fallback(
    Some(r#"[{"type":"input_image","url":"https://x/c.png"}]"#),
    r#"[{"type":"image_url","image_url":{"url":"https://x/c.png"}}]"#
)]
#[case::empty_image_url_falls_back_to_url(
    Some(
        r#"[{"type":"input_image","image_url":"","url":"u1"},{"type":"input_image","image_url":{},"url":"u2"},{"type":"input_image","image_url":null,"url":"u3"}]"#
    ),
    r#"[{"type":"image_url","image_url":{"url":"u1"}},{"type":"image_url","image_url":{"url":"u2"}},{"type":"image_url","image_url":{"url":"u3"}}]"#
)]
#[case::url_object_as_fallback(
    Some(r#"[{"type":"input_image","url":{"url":"u4"}}]"#),
    r#"[{"type":"image_url","image_url":{"url":"u4"}}]"#
)]
#[case::object_without_url_does_not_fall_back(
    Some(r#"[{"type":"input_image","image_url":{"detail":"low"},"url":"ignored"}]"#),
    r#""[{\"type\": \"input_image\", \"image_url\": {\"detail\": \"low\"}, \"url\": \"ignored\"}]""#
)]
#[case::empty_url_inside_object_ignored(
    Some(r#"[{"type":"input_image","image_url":{"url":""}}]"#),
    r#""[{\"type\": \"input_image\", \"image_url\": {\"url\": \"\"}}]""#
)]
#[case::text_and_images_keep_order(
    Some(
        r#"[{"type":"text","text":"before"},{"type":"input_image","image_url":"i1"},{"type":"output_text","text":"after"}]"#
    ),
    r#"[{"type":"text","text":"before"},{"type":"image_url","image_url":{"url":"i1"}},{"type":"text","text":"after"}]"#
)]
#[case::empty_text_skipped_beside_image(
    Some(r#"[{"type":"text","text":""},{"type":"input_image","image_url":"i1"}]"#),
    r#"[{"type":"image_url","image_url":{"url":"i1"}}]"#
)]
#[case::empty_list(Some("[]"), r#""[]""#)]
#[case::unrecognized_list(
    Some(r#"[{"type":"audio","data":"\u00e9"}]"#),
    r#""[{\"type\": \"audio\", \"data\": \"\\u00e9\"}]""#
)]
#[case::object(Some(r#"{"a":[1,2]}"#), r#""{\"a\": [1, 2]}""#)]
#[case::number(Some("7"), r#""7""#)]
#[case::boolean(Some("true"), r#""true""#)]
fn output_content(#[case] output: Option<&str>, #[case] expected: &str) {
    let parsed = output.map(parse);
    assert_eq!(wire(&tool_output_content(parsed.as_ref())), parse(expected));
}

#[rstest]
#[case::absent(r#"{"output":"x"}"#)]
#[case::null(r#"{"call_id":null}"#)]
#[case::empty(r#"{"call_id":"","output":"x"}"#)]
#[case::zero(r#"{"call_id":0,"output":"x"}"#)]
fn missing_call_id_makes_no_message(#[case] item: &str) {
    assert!(messages(item, &[("", r#"{"id":"c"}"#)]).is_empty());
}

#[test]
fn null_output_makes_an_empty_tool_message() {
    assert_eq!(
        wire(&messages(r#"{"call_id":"c","output":null}"#, &[])),
        parse(r#"[{"role":"tool","content":"","tool_call_id":"c"}]"#)
    );
}

#[test]
fn numeric_call_id_becomes_text_but_misses_the_text_keyed_cache() {
    assert_eq!(
        messages(r#"{"call_id":7,"output":"x"}"#, &[("7", r#"{"id":"7"}"#)]),
        vec![tool_message("7", "x")]
    );
    assert_eq!(
        messages(r#"{"call_id":-3,"output":"x"}"#, &[]),
        vec![tool_message("-3", "x")]
    );
}

#[test]
fn cached_definition_puts_an_assistant_call_before_the_result() {
    let cached =
        r#"{"id":"toolu_1","type":"function","function":{"name":"shell","arguments":"{\"a\":1}"}}"#;
    assert_eq!(
        wire(&messages(
            r#"{"call_id":"c","output":"done"}"#,
            &[("c", cached)]
        )),
        parse(
            r#"[
                {"role":"assistant","tool_calls":[{"id":"toolu_1","type":"function","index":0,
                  "function":{"name":"shell","arguments":"{\"a\":1}"}}]},
                {"role":"tool","content":"done","tool_call_id":"c"}
            ]"#
        )
    );
}

#[rstest]
#[case::missing_function(r#"{"id":"x"}"#, call_chunk("x", "function", "", ""))]
#[case::null_function(r#"{"id":"x","function":null}"#, call_chunk("x", "function", "", ""))]
#[case::falsy_fields(
    r#"{"id":"","type":"","function":{"name":"","arguments":""}}"#,
    call_chunk("", "function", "", "")
)]
#[case::null_arguments(
    r#"{"id":"x","function":{"name":"n","arguments":null}}"#,
    call_chunk("x", "function", "n", "")
)]
#[case::object_arguments(
    r#"{"id":"x","function":{"name":"n","arguments":{"x":1}}}"#,
    call_chunk("x", "function", "n", r#"{"x": 1}"#)
)]
#[case::other_type(
    r#"{"id":"x","type":"custom","function":{"name":"n"}}"#,
    call_chunk("x", "custom", "n", "")
)]
fn cached_call_defaults(#[case] cached: &str, #[case] expected: ToolCallChunk) {
    assert_eq!(
        messages(r#"{"call_id":"c","output":"o"}"#, &[("c", cached)]),
        vec![
            ResultMessage::CachedCall(CachedCallMessage {
                role: "assistant",
                tool_calls: vec![expected],
            }),
            tool_message("c", "o"),
        ]
    );
}

#[rstest]
#[case::empty_object("{}")]
#[case::null("null")]
fn falsy_cache_entry_makes_only_the_tool_message(#[case] cached: &str) {
    assert_eq!(
        messages(r#"{"call_id":"c","output":"o"}"#, &[("c", cached)]),
        vec![tool_message("c", "o")]
    );
}

#[test]
fn cache_is_read_by_the_exact_call_id() {
    let entries = [("c", r#"{"id":"x"}"#)];
    assert_eq!(
        messages(r#"{"call_id":"C","output":"o"}"#, &entries).len(),
        1
    );
    assert_eq!(
        messages(r#"{"call_id":"c","output":"o"}"#, &entries).len(),
        2
    );
}

#[rstest]
#[case::bool_call_id(r#"{"call_id":true}"#, &[], "call_id must be a string or an integer")]
#[case::list_call_id(r#"{"call_id":[1]}"#, &[], "call_id must be a string or an integer")]
#[case::id_not_text(r#"{"call_id":"c"}"#, &[("c", r#"{"id":5}"#)], "id must be a string")]
#[case::function_not_object(
    r#"{"call_id":"c"}"#,
    &[("c", r#"{"function":"f"}"#)],
    "function must be an object"
)]
#[case::name_not_text(
    r#"{"call_id":"c"}"#,
    &[("c", r#"{"function":{"name":3}}"#)],
    "name must be a string"
)]
#[case::cached_not_object(
    r#"{"call_id":"c"}"#,
    &[("c", r#""text""#)],
    "the cached tool call is not an object"
)]
fn invalid_input_names_the_field(
    #[case] item: &str,
    #[case] entries: &[(&str, &str)],
    #[case] expected: &str,
) {
    let error = tool_call_output_messages(&fields(item), &cache(entries)).unwrap_err();
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::namespace_joins(
    r#"{"type":"function_call","call_id":"c","id":"fc_i","name":"f","namespace":"ns","arguments":"{}"}"#,
    call_chunk("c", "function", "ns__f", "{}")
)]
#[case::call_id_falls_back_to_id(
    r#"{"id":"fc_i","name":"f","arguments":"a"}"#,
    call_chunk("fc_i", "function", "f", "a")
)]
#[case::no_ids(r#"{"name":"f"}"#, call_chunk("", "function", "f", ""))]
#[case::empty_call_id_uses_id(
    r#"{"call_id":"","id":"fc_i","name":"f"}"#,
    call_chunk("fc_i", "function", "f", "")
)]
#[case::index_shaped_call_id_is_kept(
    r#"{"call_id":"call_0","id":"fc_i","name":"f"}"#,
    call_chunk("call_0", "function", "f", "")
)]
#[case::redacted_arguments(
    r#"{"name":"f","arguments":"redacted-by-litellm"}"#,
    call_chunk("", "function", "f", "{}")
)]
#[case::redacted_arguments_of_a_custom_call(
    r#"{"type":"custom_tool_call","name":"f","arguments":"redacted-by-litellm","input":"p"}"#,
    call_chunk("", "function", "f", "{}")
)]
#[case::object_arguments(
    r#"{"name":"f","arguments":{"x":1}}"#,
    call_chunk("", "function", "f", r#"{"x": 1}"#)
)]
#[case::custom_ignores_namespace_and_wraps_input(
    r#"{"type":"custom_tool_call","call_id":"c","name":"apply","namespace":"ns","input":"patch"}"#,
    call_chunk("c", "function", "apply", r#"{"content": "patch"}"#)
)]
#[case::custom_empty_input(
    r#"{"type":"custom_tool_call","name":"a","input":""}"#,
    call_chunk("", "function", "a", "")
)]
#[case::custom_without_input(
    r#"{"type":"custom_tool_call","name":"a"}"#,
    call_chunk("", "function", "a", "")
)]
#[case::custom_empty_arguments_use_input(
    r#"{"type":"custom_tool_call","name":"a","arguments":"","input":"\u00e9"}"#,
    call_chunk("", "function", "a", r#"{"content": "\u00e9"}"#)
)]
#[case::custom_arguments_win_over_input(
    r#"{"type":"custom_tool_call","name":"a","arguments":"{\"k\":1}","input":"p"}"#,
    call_chunk("", "function", "a", r#"{"k":1}"#)
)]
#[case::plain_call_ignores_input(
    r#"{"type":"function_call","name":"a","input":"p"}"#,
    call_chunk("", "function", "a", "")
)]
#[case::empty_namespace_is_unqualified(
    r#"{"name":"a","namespace":""}"#,
    call_chunk("", "function", "a", "")
)]
fn function_call_items(#[case] item: &str, #[case] expected: ToolCallChunk) {
    let message = input_function_call(&fields(item)).unwrap();
    assert_eq!(message.tool_calls, vec![expected]);
}

#[test]
fn function_call_message_has_null_content() {
    let message = input_function_call(&fields(r#"{"call_id":"c","name":"f"}"#)).unwrap();
    assert_eq!(
        wire(&message),
        parse(
            r#"{"role":"assistant","content":null,"tool_calls":[{"id":"c","type":"function","index":0,
              "function":{"name":"f","arguments":""}}]}"#
        )
    );
}

#[test]
fn function_call_input_names_a_field_of_the_wrong_type() {
    let name_error = input_function_call(&fields(r#"{"name":5}"#)).unwrap_err();
    assert_eq!(name_error.to_string(), "name must be a string");
    let input_error =
        input_function_call(&fields(r#"{"type":"custom_tool_call","input":5}"#)).unwrap_err();
    assert_eq!(input_error.to_string(), "input must be a string");
}

#[rstest]
#[case::unique_call_id_wins(Some("fc_i"), Some("call_unique"), "call_unique")]
#[case::index_shaped_uses_item_id(Some("fc_i"), Some("call_0"), "fc_i")]
#[case::multi_digit_index(Some("fc_i"), Some("call_123"), "fc_i")]
#[case::absent_call_id(Some("fc_i"), None, "fc_i")]
#[case::empty_call_id(Some("fc_i"), Some(""), "fc_i")]
#[case::index_shape_without_item_id(None, Some("call_0"), "call_0")]
#[case::index_shape_with_empty_item_id(Some(""), Some("call_0"), "call_0")]
#[case::both_absent(None, None, "")]
#[case::unicode_digits_are_an_index(Some("fc_i"), Some("call_\u{663}"), "fc_i")]
#[case::trailing_text_is_unique(Some("fc_i"), Some("call_1a"), "call_1a")]
#[case::no_digits_is_unique(Some("fc_i"), Some("call_"), "call_")]
#[case::prefix_must_match(Some("fc_i"), Some("xcall_1"), "xcall_1")]
#[case::newline_is_unique(Some("fc_i"), Some("call_1\n"), "call_1\n")]
fn call_identity(
    #[case] item_id: Option<&str>,
    #[case] call_id: Option<&str>,
    #[case] expected: &str,
) {
    let identity = CallIdentity::new().unwrap();
    assert_eq!(identity.tool_call_id(item_id, call_id), expected);
}

fn tool_call(
    id: Option<&str>,
    call_id: Option<&str>,
    provider: Option<&AttributeValue>,
) -> Result<ToolCallChunk, ToolResultsError> {
    let item = FunctionToolCall {
        name: "shell",
        arguments: "{\"a\":1}",
        id,
        call_id,
        provider_specific_fields: provider,
    };
    CallIdentity::new()?.function_tool_call(&item)
}

#[test]
fn function_tool_call_copies_name_and_arguments_with_index_zero() {
    assert_eq!(
        tool_call(Some("fc_i"), Some("call_0"), None).unwrap(),
        call_chunk("fc_i", "function", "shell", "{\"a\":1}")
    );
    assert_eq!(tool_call(None, None, None).unwrap().id, "");
}

#[rstest]
#[case::dict(AttributeValue::Plain(parse(r#"{"thought_signature":"abc"}"#)))]
#[case::attribute_object(AttributeValue::Record(vec![parse(r#"["thought_signature","abc"]"#)]))]
fn function_tool_call_keeps_truthy_provider_fields_in_both_places(
    #[case] provider: AttributeValue,
) {
    let converted = tool_call(Some("fc_i"), Some("c"), Some(&provider)).unwrap();
    let expected = fields(r#"{"thought_signature":"abc"}"#);
    assert_eq!(converted.provider_specific_fields, Some(expected.clone()));
    assert_eq!(converted.function.provider_specific_fields, Some(expected));
}

#[rstest]
#[case::empty_dict(AttributeValue::Plain(parse("{}")))]
#[case::null(AttributeValue::Plain(Json::Null))]
#[case::text_has_no_attributes(AttributeValue::Plain(parse(r#""x""#)))]
#[case::empty_record(AttributeValue::Record(Vec::new()))]
fn function_tool_call_drops_falsy_provider_fields(#[case] provider: AttributeValue) {
    let converted = tool_call(Some("fc_i"), Some("c"), Some(&provider)).unwrap();
    assert_eq!(converted, call_chunk("c", "function", "shell", "{\"a\":1}"));
}

#[test]
fn function_tool_call_without_provider_fields_serializes_neither_key() {
    let converted = tool_call(Some("fc_i"), Some("c"), None).unwrap();
    assert_eq!(
        wire(&converted),
        parse(
            r#"{"id":"c","type":"function","index":0,"function":{"name":"shell","arguments":"{\"a\":1}"}}"#
        )
    );
}

#[test]
fn function_tool_call_rejects_a_record_that_is_not_pairs() {
    let provider = AttributeValue::Record(vec![parse("5")]);
    let error = tool_call(Some("fc_i"), Some("c"), Some(&provider)).unwrap_err();
    assert!(matches!(error, ToolResultsError::Attributes { .. }));
}

#[rstest]
#[case::create(
    r#"{"type":"create_file","path":"a.rs","diff":"+x"}"#,
    r#"{"diff": "+x", "path": "a.rs", "type": "create_file"}"#
)]
#[case::update(
    r#"{"type":"update_file","path":"b.rs","diff":"@@\u00e9"}"#,
    r#"{"diff": "@@\u00e9", "path": "b.rs", "type": "update_file"}"#
)]
#[case::delete(
    r#"{"type":"delete_file","path":"c.rs"}"#,
    r#"{"path": "c.rs", "type": "delete_file"}"#
)]
fn apply_patch_uses_the_call_id_and_keeps_the_index(
    #[case] operation_text: &str,
    #[case] arguments: &str,
) {
    let operation = serde_json::from_str::<ApplyPatchOperation>(operation_text).unwrap();
    let converted = apply_patch_tool_call("call_0", &operation, 4);
    let mut expected = call_chunk("call_0", "function", "apply_patch", arguments);
    expected.index = 4;
    assert_eq!(converted, expected);
}
