#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde::Serialize;

use super::custom_tools::{
    ToolCall, custom_to_function_tool, extract_custom_tool_names, serialize_tool_call_arguments,
    tool_call_item, tool_call_item_id, unwrap_custom_arguments,
};
use super::error::CustomToolsError;
use super::records::{AttributeValue, attribute_fields, input_item_role};
use super::tool_choices::{chat_tool_choice, responses_tool_choice};
use super::{drop_unsupported_tools, hoist_additional_tools, normalize_codex_input_items};
use crate::api::api_boundaries::ApiLimits;

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn fields(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn serialized<Item>(value: &Item) -> Json
where
    Item: Serialize,
{
    parse(&serde_json::to_string(value).unwrap())
}

fn names(text: &str) -> Vec<String> {
    let Json::List(tools) = parse(text) else {
        panic!("tools must be a list");
    };
    extract_custom_tool_names(&tools).into_iter().collect()
}

#[rstest]
#[case::top_level_and_one_namespace_level(
    r#"[{"type":"custom","name":"edit"},{"type":"namespace","name":"ns","tools":[{"type":"custom","name":"inner"},{"type":"namespace","tools":[{"type":"custom","name":"deep"}]}]}]"#,
    &["edit", "inner"]
)]
#[case::empty_names_and_other_types(
    r#"[{"type":"custom","name":""},{"type":"custom"},{"type":"function","name":"f"},{"name":"untyped"}]"#,
    &[]
)]
#[case::invalid_records(
    r#"[7,"custom",null,{"type":5,"name":"a"},{"type":"custom","name":null},{"type":"custom","name":"b","tools":7},{"type":"custom","name":"c","tools":null}]"#,
    &[]
)]
#[case::record_with_valid_tools_field(r#"[{"type":"custom","name":"edit","tools":[]}]"#, &["edit"])]
#[case::namespace_with_invalid_member(
    r#"[{"type":"namespace","tools":[3,{"type":"custom","name":"kept"}]}]"#,
    &["kept"]
)]
fn custom_names(#[case] tools: &str, #[case] expected: &[&str]) {
    assert_eq!(names(tools), expected);
}

#[test]
fn custom_names_are_exact_and_case_sensitive() {
    let custom = extract_custom_tool_names(&[parse(r#"{"type":"custom","name":"edit"}"#)]);
    assert!(custom.contains("edit"));
    assert!(!custom.contains("Edit"));
    assert!(!custom.contains("edit "));
}

#[rstest]
#[case::function_call("function_call", "call_a", "fc_call_a")]
#[case::custom_tool_call("custom_tool_call", "call_a", "ctc_call_a")]
#[case::empty_id("function_call", "", "")]
#[case::unknown_type("message", "call_a", "call_a")]
#[case::already_fc("function_call", "fc_a", "fc_a")]
#[case::fc_without_underscore("function_call", "fcabc", "fcabc")]
#[case::ctc_without_underscore("custom_tool_call", "ctcabc", "ctcabc")]
#[case::ctc_prefix_on_function_call("function_call", "ctc_a", "fc_ctc_a")]
fn item_ids(#[case] item_type: &str, #[case] tool_id: &str, #[case] expected: &str) {
    assert_eq!(tool_call_item_id(item_type, tool_id), expected);
}

fn schema(tool: &str) -> Result<Json, CustomToolsError> {
    let converted = custom_to_function_tool(&fields(tool))?;
    Ok(converted.map_or(Json::Null, |function_tool| serialized(&function_tool)))
}

#[test]
fn custom_schema_embeds_the_grammar() {
    let tool = r#"{"type":"custom","name":"edit","description":"Apply","format":{"syntax":"lark","definition":"start"},"allowed_callers":["direct"]}"#;
    let expected = r#"{"type":"function","allowed_callers":["direct"],"function":{"name":"edit","description":"Apply\n\nFormat:\n```lark\nstart\n```","parameters":{"type":"object","properties":{"content":{"type":"string","description":"The edit content following the specified format"}},"required":["content"]}}}"#;
    assert_eq!(schema(tool).unwrap(), parse(expected));
}

#[rstest]
#[case::missing_name_and_description(r#"{"type":"custom"}"#, "", "")]
#[case::non_string_name_and_description(r#"{"type":"custom","name":5,"description":null}"#, "", "")]
#[case::no_format(r#"{"type":"custom","name":"e","description":"D"}"#, "e", "D")]
#[case::non_object_format(
    r#"{"type":"custom","name":"e","description":"D","format":"lark"}"#,
    "e",
    "D"
)]
#[case::empty_definition(
    r#"{"type":"custom","name":"e","description":"D","format":{"syntax":"lark","definition":""}}"#,
    "e",
    "D"
)]
#[case::missing_definition(
    r#"{"type":"custom","name":"e","description":"D","format":{"syntax":"lark"}}"#,
    "e",
    "D"
)]
#[case::wrong_typed_syntax(
    r#"{"type":"custom","name":"e","description":"D","format":{"syntax":3,"definition":"x"}}"#,
    "e",
    "D"
)]
#[case::missing_syntax(
    r#"{"type":"custom","name":"e","format":{"definition":"x"}}"#,
    "e",
    "\n\nFormat:\n```\nx\n```"
)]
fn custom_schema_defaults(#[case] tool: &str, #[case] name: &str, #[case] description: &str) {
    let Json::Object(converted) = schema(tool).unwrap() else {
        panic!("a custom tool converts");
    };
    let Some(Json::Object(function)) = converted.get("function") else {
        panic!("function chunk");
    };
    assert_eq!(function.get("name"), Some(&Json::Text(name.to_owned())));
    assert_eq!(
        function.get("description"),
        Some(&Json::Text(description.to_owned()))
    );
    assert!(!converted.contains_key("allowed_callers"));
}

#[rstest]
#[case::empty_list("[]", true)]
#[case::null("null", false)]
fn custom_schema_allowed_callers(#[case] callers: &str, #[case] present: bool) {
    let tool = format!(r#"{{"type":"custom","name":"e","allowed_callers":{callers}}}"#);
    let Json::Object(converted) = schema(&tool).unwrap() else {
        panic!("a custom tool converts");
    };
    assert_eq!(converted.get("allowed_callers").is_some(), present);
}

#[rstest]
#[case::string(r#""direct""#)]
#[case::number_item("[1]")]
#[case::null_item("[null]")]
#[case::object(r#"{"a":"b"}"#)]
#[case::number("3")]
fn custom_schema_rejects_bad_allowed_callers(#[case] callers: &str) {
    let tool = format!(r#"{{"type":"custom","name":"e","allowed_callers":{callers}}}"#);
    assert_eq!(schema(&tool), Err(CustomToolsError::AllowedCallers));
}

#[rstest]
#[case::function(r#"{"type":"function","name":"f"}"#)]
#[case::missing_type(r#"{"name":"f"}"#)]
#[case::non_string_type(r#"{"type":5}"#)]
fn non_custom_tools_do_not_convert(#[case] tool: &str) {
    assert_eq!(schema(tool), Ok(Json::Null));
}

#[rstest]
#[case::object(r#"{"x":1}"#, "", r#"{"x": 1}"#)]
#[case::nested_and_escaped("{\"\u{e9}\":[true,null]}", "", "{\"\\u00e9\": [true, null]}")]
#[case::null("null", "d", "d")]
#[case::empty_string(r#""""#, "d", "d")]
#[case::default_is_empty("null", "", "")]
#[case::plain_string(r#""{\"a\":1}""#, "d", r#"{"a":1}"#)]
#[case::number("7", "d", "7")]
fn argument_strings(#[case] raw: &str, #[case] default: &str, #[case] expected: &str) {
    assert_eq!(
        serialize_tool_call_arguments(&parse(raw), default),
        expected
    );
}

fn unwrap(arguments: &str) -> String {
    unwrap_custom_arguments(arguments.as_bytes(), ApiLimits::default()).unwrap()
}

#[rstest]
#[case::string(r#"{"content":"patch"}"#, "patch")]
#[case::null(r#"{"content":null}"#, "None")]
#[case::number(r#"{"content":7}"#, "7")]
#[case::float(r#"{"content":1.5}"#, "1.5")]
#[case::true_value(r#"{"content":true}"#, "True")]
#[case::false_value(r#"{"content":false}"#, "False")]
#[case::empty_content(r#"{"content":""}"#, "")]
#[case::empty("", "")]
#[case::duplicate_key_takes_last(r#"{"content":"a","content":"b"}"#, "b")]
#[case::malformed(r#"{"content":"#, r#"{"content":"#)]
#[case::array(r#"["content"]"#, r#"["content"]"#)]
#[case::no_content(r#"{"other":"x"}"#, r#"{"other":"x"}"#)]
#[case::plain_text("plain", "plain")]
fn unwrap_cases(#[case] arguments: &str, #[case] expected: &str) {
    assert_eq!(unwrap(arguments), expected);
}

fn padded(fill: &str, count: usize) -> String {
    format!(r#"{{"content":"{}"}}"#, fill.repeat(count))
}

#[rstest]
#[case::ascii("a", 999_986, true)]
#[case::ascii_over("a", 999_987, false)]
#[case::unicode("\u{e9}", 999_986, true)]
#[case::unicode_over("\u{e9}", 999_987, false)]
#[case::astral("\u{1f600}", 999_986, true)]
#[case::astral_over("\u{1f600}", 999_987, false)]
fn unwrap_threshold_counts_characters(
    #[case] fill: &str,
    #[case] count: usize,
    #[case] parsed: bool,
) {
    let arguments = padded(fill, count);
    let result = unwrap(&arguments);
    let total = if parsed { 1_000_000 } else { 1_000_001 };
    assert_eq!(arguments.chars().count(), total);
    if parsed {
        assert_eq!(result, fill.repeat(count));
    } else {
        assert_eq!(result, arguments);
    }
}

#[test]
fn unwrap_applies_the_outer_byte_bound_before_the_character_bound() {
    let limits = ApiLimits {
        max_json_bytes: 16,
        max_event_bytes: 16,
        max_items: 16,
    };
    let error = unwrap_custom_arguments(b"{\"content\":\"abc\"}", limits).unwrap_err();
    assert!(matches!(error, CustomToolsError::ArgumentsTooLarge { .. }));
    assert_eq!(
        unwrap_custom_arguments(b"{\"content\":\"a\"}", limits),
        Ok("a".to_owned())
    );
}

#[test]
fn unwrap_rejects_bytes_that_are_not_utf8() {
    let error = unwrap_custom_arguments(&[0x7b, 0xff], ApiLimits::default()).unwrap_err();
    assert!(matches!(error, CustomToolsError::ArgumentsNotUtf8 { .. }));
}

fn item(name: &str, input: &str, status: &str) -> Json {
    let custom = BTreeSet::from(["edit".to_owned()]);
    let call = ToolCall {
        call_id: "call_a",
        name,
        arguments_or_input: input,
        status,
    };
    serialized(&tool_call_item(call, &custom, ApiLimits::default()).unwrap())
}

#[rstest]
#[case::completed_custom(
    "edit",
    r#"{"content":"patch"}"#,
    "completed",
    r#"{"type":"custom_tool_call","id":"ctc_call_a","call_id":"call_a","name":"edit","status":"completed","input":"patch"}"#
)]
#[case::partial_custom(
    "edit",
    r#"{"content":"patch"}"#,
    "in_progress",
    r#"{"type":"custom_tool_call","id":"ctc_call_a","call_id":"call_a","name":"edit","status":"in_progress","input":""}"#
)]
#[case::ordinary_completed(
    "other",
    r#"{"content":"patch"}"#,
    "completed",
    r#"{"type":"function_call","id":"fc_call_a","call_id":"call_a","name":"other","status":"completed","arguments":"{\"content\":\"patch\"}"}"#
)]
#[case::ordinary_partial(
    "other",
    "{\"a",
    "in_progress",
    r#"{"type":"function_call","id":"fc_call_a","call_id":"call_a","name":"other","status":"in_progress","arguments":"{\"a"}"#
)]
#[case::name_is_case_sensitive(
    "Edit",
    "x",
    "completed",
    r#"{"type":"function_call","id":"fc_call_a","call_id":"call_a","name":"Edit","status":"completed","arguments":"x"}"#
)]
fn tool_call_items(
    #[case] name: &str,
    #[case] input: &str,
    #[case] status: &str,
    #[case] expected: &str,
) {
    assert_eq!(item(name, input, status), parse(expected));
}

#[rstest]
#[case::null("null", "null")]
#[case::string(r#""anything""#, r#""anything""#)]
#[case::number("3", "3")]
#[case::auto_dict(r#"{"type":"auto"}"#, r#""auto""#)]
#[case::none_dict(r#"{"type":"none"}"#, r#""none""#)]
#[case::required_dict(r#"{"type":"required"}"#, r#""required""#)]
#[case::tool_dict(r#"{"type":"tool"}"#, r#""required""#)]
#[case::any_dict(r#"{"type":"any"}"#, r#""required""#)]
#[case::function_name_keeps_dict(
    r#"{"type":"auto","function":{"name":"f"},"extra":1}"#,
    r#"{"type":"auto","function":{"name":"f"},"extra":1}"#
)]
#[case::function_without_name_is_ignored(r#"{"type":"auto","function":{"name":""}}"#, r#""auto""#)]
#[case::function_type(
    r#"{"type":"function","name":"f"}"#,
    r#"{"type":"function","function":{"name":"f"}}"#
)]
#[case::function_without_name(r#"{"type":"function"}"#, r#""required""#)]
#[case::function_empty_name(r#"{"type":"function","name":""}"#, r#""required""#)]
#[case::custom_top_level_name_first(
    r#"{"type":"custom","name":"top","custom":{"name":"inner"}}"#,
    r#"{"type":"function","function":{"name":"top"}}"#
)]
#[case::custom_inner_name(
    r#"{"type":"custom","custom":{"name":"inner"}}"#,
    r#"{"type":"function","function":{"name":"inner"}}"#
)]
#[case::custom_empty_top_name_falls_back(
    r#"{"type":"custom","name":"","custom":{"name":"inner"}}"#,
    r#"{"type":"function","function":{"name":"inner"}}"#
)]
#[case::custom_without_name(r#"{"type":"custom"}"#, r#""required""#)]
#[case::custom_non_object(r#"{"type":"custom","custom":"x"}"#, r#""required""#)]
#[case::unknown_type(r#"{"type":"mystery","a":1}"#, r#"{"type":"mystery","a":1}"#)]
#[case::missing_type(r#"{"a":1}"#, r#"{"a":1}"#)]
fn request_tool_choices(#[case] choice: &str, #[case] expected: &str) {
    assert_eq!(chat_tool_choice(&parse(choice)), parse(expected));
}

#[rstest]
#[case::null("null", r#""auto""#)]
#[case::none(r#""none""#, r#""none""#)]
#[case::auto(r#""auto""#, r#""auto""#)]
#[case::required(r#""required""#, r#""required""#)]
#[case::unknown_string(r#""sometimes""#, r#""auto""#)]
#[case::number("3", r#""auto""#)]
#[case::function_extras_dropped(
    r#"{"type":"function","name":"f","extra":1}"#,
    r#"{"type":"function","name":"f"}"#
)]
#[case::custom_kept(
    r#"{"type":"custom","name":"edit"}"#,
    r#"{"type":"custom","name":"edit"}"#
)]
#[case::empty_function_name_is_a_valid_choice(
    r#"{"type":"function","name":""}"#,
    r#"{"type":"function","name":""}"#
)]
#[case::chat_function(
    r#"{"type":"function","function":{"name":"f"}}"#,
    r#"{"type":"function","name":"f"}"#
)]
#[case::chat_custom_keeps_custom(
    r#"{"type":"custom","custom":{"name":"edit"}}"#,
    r#"{"type":"custom","name":"edit"}"#
)]
#[case::chat_custom_with_function_name(
    r#"{"type":"custom","function":{"name":"edit"}}"#,
    r#"{"type":"custom","name":"edit"}"#
)]
#[case::cursor_dict(r#"{"type":"tool"}"#, r#""required""#)]
#[case::cursor_auto(r#"{"type":"auto"}"#, r#""auto""#)]
#[case::function_without_name(r#"{"type":"function"}"#, r#""required""#)]
#[case::unusual_type_with_function_name(r#"{"type":"tool","function":{"name":"f"}}"#, r#""auto""#)]
#[case::non_string_function_name(r#"{"type":"function","function":{"name":5}}"#, r#""auto""#)]
#[case::unknown_dict(r#"{"type":"mystery"}"#, r#""auto""#)]
#[case::file_search(r#"{"type":"file_search","extra":true}"#, r#"{"type":"file_search"}"#)]
#[case::apply_patch(r#"{"type":"apply_patch"}"#, r#"{"type":"apply_patch"}"#)]
#[case::shell(r#"{"type":"shell"}"#, r#"{"type":"shell"}"#)]
#[case::programmatic(
    r#"{"type":"programmatic_tool_calling"}"#,
    r#"{"type":"programmatic_tool_calling"}"#
)]
#[case::mcp(
    r#"{"type":"mcp","server_label":"s"}"#,
    r#"{"type":"mcp","server_label":"s"}"#
)]
#[case::mcp_named(
    r#"{"type":"mcp","server_label":"s","name":"t"}"#,
    r#"{"type":"mcp","server_label":"s","name":"t"}"#
)]
#[case::mcp_null_name(
    r#"{"type":"mcp","server_label":"s","name":null}"#,
    r#"{"type":"mcp","server_label":"s","name":null}"#
)]
#[case::mcp_bad_name(r#"{"type":"mcp","server_label":"s","name":3}"#, r#""auto""#)]
#[case::mcp_without_label(r#"{"type":"mcp"}"#, r#""auto""#)]
#[case::allowed_tools(
    r#"{"type":"allowed_tools","mode":"auto","tools":[{"type":"function","name":"f"}],"extra":1}"#,
    r#"{"type":"allowed_tools","mode":"auto","tools":[{"type":"function","name":"f"}]}"#
)]
#[case::allowed_tools_bad_mode(r#"{"type":"allowed_tools","mode":"none","tools":[]}"#, r#""auto""#)]
#[case::allowed_tools_bad_member(
    r#"{"type":"allowed_tools","mode":"auto","tools":[1]}"#,
    r#""auto""#
)]
#[case::allowed_tools_without_tools(r#"{"type":"allowed_tools","mode":"auto"}"#, r#""auto""#)]
fn response_tool_choices(#[case] choice: &str, #[case] expected: &str) {
    assert_eq!(responses_tool_choice(&parse(choice)), parse(expected));
}

#[rstest]
#[case::web_search("web_search_preview")]
#[case::computer("computer")]
#[case::computer_use_preview("computer_use_preview")]
#[case::computer_use("computer_use")]
#[case::dated_web_search("web_search_preview_2025_03_11")]
#[case::image_generation("image_generation")]
#[case::code_interpreter("code_interpreter")]
fn hosted_tool_types_are_valid_responses_choices(#[case] kind: &str) {
    let choice = parse(&format!(r#"{{"type":"{kind}","extra":1}}"#));
    assert_eq!(
        responses_tool_choice(&choice),
        parse(&format!(r#"{{"type":"{kind}"}}"#))
    );
}

#[rstest]
#[case::plain_text(AttributeValue::Plain(Json::Text("x".to_owned())), Ok("{}"))]
#[case::plain_object(AttributeValue::Plain(parse(r#"{"a":1}"#)), Ok("{}"))]
#[case::plain_null(AttributeValue::Plain(Json::Null), Ok("{}"))]
#[case::pairs(AttributeValue::Record(vec![parse(r#"["a",1]"#), parse(r#"["b",null]"#)]), Ok(r#"{"a":1,"b":null}"#))]
#[case::empty_record(AttributeValue::Record(Vec::new()), Ok("{}"))]
#[case::later_pair_wins(AttributeValue::Record(vec![parse(r#"["a",1]"#), parse(r#"["a",2]"#)]), Ok(r#"{"a":2}"#))]
#[case::not_a_pair(
    AttributeValue::Record(vec![parse(r#"["a",1]"#), parse("1")]),
    Err(CustomToolsError::NotAPair { position: 1 })
)]
#[case::too_long(AttributeValue::Record(vec![parse(r#"["a",1,2]"#)]), Err(CustomToolsError::NotAPair { position: 0 }))]
#[case::too_short(AttributeValue::Record(vec![parse(r#"["a"]"#)]), Err(CustomToolsError::NotAPair { position: 0 }))]
#[case::key_not_text(AttributeValue::Record(vec![parse("[1,2]")]), Err(CustomToolsError::NotAPair { position: 0 }))]
fn attribute_records(
    #[case] value: AttributeValue,
    #[case] expected: Result<&str, CustomToolsError>,
) {
    assert_eq!(attribute_fields(&value), expected.map(fields));
}

#[rstest]
#[case::missing("{}", Ok("user"))]
#[case::null(r#"{"role":null}"#, Ok("user"))]
#[case::empty(r#"{"role":""}"#, Ok("user"))]
#[case::assistant(r#"{"role":"assistant"}"#, Ok("assistant"))]
#[case::unusual(r#"{"role":"banana"}"#, Ok("banana"))]
#[case::not_text(r#"{"role":5}"#, Err(CustomToolsError::RoleNotText))]
#[case::list_role(r#"{"role":["user"]}"#, Err(CustomToolsError::RoleNotText))]
fn input_roles(#[case] item: &str, #[case] expected: Result<&str, CustomToolsError>) {
    assert_eq!(input_item_role(&fields(item)), expected.map(str::to_owned));
}

#[test]
fn codex_history_preparation_is_reachable_from_this_module() {
    let input = serde_json::json!("hi");
    assert_eq!(
        normalize_codex_input_items(&input),
        (input.clone(), Vec::new())
    );
    assert!(hoist_additional_tools(&input, None).hoisted.is_empty());
    assert_eq!(
        drop_unsupported_tools(&[], &BTreeSet::new()),
        (Vec::new(), Vec::new())
    );
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/responses_codex_tools");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let result = unwrap_custom_arguments(&fs::read(&path).unwrap(), ApiLimits::default());
            (path.file_name().unwrap().to_owned(), result)
        })
        .collect::<Vec<_>>();
    assert!(replayed.len() >= 12);
    assert!(
        replayed
            .iter()
            .any(|(name, result)| name == "string-content" && result.as_deref() == Ok("patch"))
    );
}
