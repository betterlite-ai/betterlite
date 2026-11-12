#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::FunctionCallStatus;
use rstest::rstest;
use serde::Serialize;

use super::error::NamespaceToolsError;
use super::hosted_search::calls_by_id;
use super::namespace_names::{check_collisions, name_map, restore_name};
use super::records::{ChoiceRecord, FunctionRecord, ProviderFields, ToolCallRecord};
use super::responses_tools::{chat_tools_to_responses, responses_tools_to_chat};
use super::tool_outputs::tool_outputs;
use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_codex_tools::records::AttributeValue;

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn tools(text: &str) -> Vec<Json> {
    let Json::List(items) = parse(text) else {
        panic!("tools must be a list");
    };
    items
}

fn serialized<Item>(value: &Item) -> Json
where
    Item: Serialize,
{
    parse(&serde_json::to_string(value).unwrap())
}

fn chat_tools(text: &str) -> Json {
    serialized(&responses_tools_to_chat(&tools(text)).unwrap().chat_tools)
}

const NS_FUNCTION: &str = r#"[{"type":"namespace","name":"ns","description":"Group","tools":[{"type":"function","name":"f","description":"Fn"}]}]"#;

#[rstest]
#[case::joined_description(
    NS_FUNCTION,
    r#"[{"type":"function","function":{"name":"ns__f","description":"Group\n\nFn","parameters":{"type":"object"},"strict":false}}]"#
)]
#[case::missing_member_description_uses_group(
    r#"[{"type":"namespace","name":"ns","description":"Group","tools":[{"type":"function","name":"f"}]}]"#,
    r#"[{"type":"function","function":{"name":"ns__f","description":"Group","parameters":{"type":"object"},"strict":false}}]"#
)]
#[case::missing_group_description_uses_member(
    r#"[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"f","description":"Fn"}]}]"#,
    r#"[{"type":"function","function":{"name":"ns__f","description":"Fn","parameters":{"type":"object"},"strict":false}}]"#
)]
#[case::custom_member_keeps_its_name(
    r#"[{"type":"namespace","name":"ns","description":"Group","tools":[{"type":"custom","name":"edit","description":"Edit"}]}]"#,
    r#"[{"type":"function","function":{"name":"edit","description":"Group\n\nEdit","parameters":{"type":"object","properties":{"content":{"type":"string","description":"The edit content following the specified format"}},"required":["content"]}}}]"#
)]
#[case::other_member_types_are_dropped(
    r#"[{"type":"namespace","name":"ns","tools":[{"type":"web_search"},{"type":"namespace","name":"in","tools":[]},7,{"name":"untyped"}]}]"#,
    "[]"
)]
#[case::string_tools_use_the_flat_fallback(
    r#"[{"type":"namespace","name":"ns","description":"D","tools":"abc"}]"#,
    r#"[{"type":"function","function":{"name":"ns","description":"D","parameters":{"type":"object"},"strict":false}}]"#
)]
#[case::missing_tools_use_the_flat_fallback(
    r#"[{"type":"namespace","name":"ns","strict":true}]"#,
    r#"[{"type":"function","function":{"name":"ns","description":"","parameters":{"type":"object"},"strict":true}}]"#
)]
#[case::null_tools_use_the_flat_fallback(
    r#"[{"type":"namespace","name":"ns","tools":null}]"#,
    r#"[{"type":"function","function":{"name":"ns","description":"","parameters":{"type":"object"},"strict":false}}]"#
)]
#[case::non_mapping_parameters_become_an_object_schema(
    r#"[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"a","parameters":null},{"type":"function","name":"b","parameters":"x"},{"type":"function","name":"c","parameters":[1]}]}]"#,
    r#"[{"type":"function","function":{"name":"ns__a","description":"","parameters":{"type":"object"},"strict":false}},{"type":"function","function":{"name":"ns__b","description":"","parameters":{"type":"object"},"strict":false}},{"type":"function","function":{"name":"ns__c","description":"","parameters":{"type":"object"},"strict":false}}]"#
)]
#[case::mapping_parameters_keep_their_type(
    r#"[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"a","parameters":{"type":"string"}},{"type":"function","name":"b","parameters":{"x":1}}]}]"#,
    r#"[{"type":"function","function":{"name":"ns__a","description":"","parameters":{"type":"string"},"strict":false}},{"type":"function","function":{"name":"ns__b","description":"","parameters":{"x":1,"type":"object"},"strict":false}}]"#
)]
#[case::strict_follows_truthiness(
    r#"[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"a","strict":"yes"},{"type":"function","name":"b","strict":0},{"type":"function","name":"c","strict":{}},{"type":"function","name":"d","strict":true}]}]"#,
    r#"[{"type":"function","function":{"name":"ns__a","description":"","parameters":{"type":"object"},"strict":true}},{"type":"function","function":{"name":"ns__b","description":"","parameters":{"type":"object"},"strict":false}},{"type":"function","function":{"name":"ns__c","description":"","parameters":{"type":"object"},"strict":false}},{"type":"function","function":{"name":"ns__d","description":"","parameters":{"type":"object"},"strict":true}}]"#
)]
#[case::empty_allowed_callers_are_kept(
    r#"[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"a","allowed_callers":[]},{"type":"function","name":"b","allowed_callers":["x"]},{"type":"function","name":"c","allowed_callers":null}]}]"#,
    r#"[{"type":"function","function":{"name":"ns__a","description":"","parameters":{"type":"object"},"strict":false},"allowed_callers":[]},{"type":"function","function":{"name":"ns__b","description":"","parameters":{"type":"object"},"strict":false},"allowed_callers":["x"]},{"type":"function","function":{"name":"ns__c","description":"","parameters":{"type":"object"},"strict":false}}]"#
)]
fn namespace_flatten(#[case] request: &str, #[case] expected: &str) {
    assert_eq!(chat_tools(request), parse(expected));
}

#[rstest]
#[case::string(r#"[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"a","allowed_callers":"x"}]}]"#)]
#[case::number_item(r#"[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"a","allowed_callers":[1]}]}]"#)]
fn namespace_flatten_rejects_invalid_allowed_callers(#[case] request: &str) {
    let error = responses_tools_to_chat(&tools(request)).unwrap_err();
    assert!(matches!(error, NamespaceToolsError::Custom(_)), "{error}");
}

#[rstest]
#[case::name(r#"[{"type":"function","name":5}]"#, "name")]
#[case::description(r#"[{"type":"function","description":[]}]"#, "description")]
#[case::parameters(r#"[{"type":"function","parameters":"x"}]"#, "parameters")]
#[case::namespace_name(r#"[{"type":"namespace","name":1}]"#, "name")]
fn wrong_typed_fields_name_the_field(#[case] request: &str, #[case] field: &str) {
    let error = responses_tools_to_chat(&tools(request)).unwrap_err();
    assert!(
        matches!(error, NamespaceToolsError::WrongType { field: named, .. } if named == field),
        "{error}"
    );
}

#[test]
fn a_tool_that_is_not_an_object_is_rejected_with_its_position() {
    let error = responses_tools_to_chat(&tools(r#"[{"type":"mcp"},"web_search"]"#)).unwrap_err();
    assert!(matches!(
        error,
        NamespaceToolsError::ToolNotObject { position: 1 }
    ));
}

#[rstest]
#[case::one(
    r#"[{"type":"function","name":"ns__f"},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"f"}]}]"#,
    "ns__f"
)]
#[case::sorted_and_joined(
    r#"[{"type":"function","name":"b__y"},{"type":"function","name":"a__x"},{"type":"namespace","name":"b","tools":[{"type":"function","name":"y"}]},{"type":"namespace","name":"a","tools":[{"type":"function","name":"x"}]}]"#,
    "a__x, b__y"
)]
#[case::repeated_conflict_listed_once(
    r#"[{"type":"function","name":"ns__f"},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"f"}]},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"f"}]}]"#,
    "ns__f"
)]
fn namespace_collisions(#[case] request: &str, #[case] names: &str) {
    let expected =
        format!("Top-level function names conflict with flattened namespace tools: {names}");
    assert_eq!(
        check_collisions(&tools(request)).unwrap_err().to_string(),
        expected
    );
    assert_eq!(
        responses_tools_to_chat(&tools(request))
            .unwrap_err()
            .to_string(),
        expected
    );
}

#[rstest]
#[case::bare_top_level_function(r#"[{"type":"function","name":"f"},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"f"}]}]"#)]
#[case::custom_member_is_excluded(r#"[{"type":"function","name":"ns__edit"},{"type":"namespace","name":"ns","tools":[{"type":"custom","name":"edit"}]}]"#)]
#[case::namespace_without_a_list(
    r#"[{"type":"function","name":"ns__f"},{"type":"namespace","name":"ns","tools":"x"}]"#
)]
#[case::member_in_a_namespace_that_is_not_a_function(r#"[{"type":"function","name":"ns__f"},{"type":"web_search","name":"ns","tools":[{"type":"function","name":"f"}]}]"#)]
fn namespace_collisions_ignore_other_shapes(#[case] request: &str) {
    check_collisions(&tools(request)).unwrap();
}

const TWO_NAMESPACES: &str = r#"[{"type":"namespace","name":"a","tools":[{"type":"function","name":"f"},{"type":"function","name":"only"},{"type":"custom","name":"edit"}]},{"type":"namespace","name":"b","tools":[{"type":"function","name":"f"}]},{"type":"function","name":"only"}]"#;

#[rstest]
#[case::qualified("a__f", "f", Some("a"))]
#[case::second_namespace("b__f", "f", Some("b"))]
#[case::ambiguous_bare_name("f", "f", None)]
#[case::bare_name_shadowed_by_a_top_level_function("only", "only", None)]
#[case::qualified_name_of_a_shadowed_function("a__only", "only", Some("a"))]
#[case::custom_member_is_not_mapped("edit", "edit", None)]
#[case::qualified_custom_member_is_not_mapped("a__edit", "a__edit", None)]
#[case::unknown_name("other", "other", None)]
fn namespace_name_restoration(
    #[case] name: &str,
    #[case] restored: &str,
    #[case] namespace: Option<&str>,
) {
    let names = name_map(&tools(TWO_NAMESPACES)).unwrap();
    assert_eq!(
        restore_name(name, &names),
        (restored.to_owned(), namespace.map(str::to_owned))
    );
}

#[test]
fn a_bare_name_restores_when_one_namespace_has_it() {
    let names = name_map(&tools(NS_FUNCTION)).unwrap();
    assert_eq!(
        restore_name("f", &names),
        ("f".to_owned(), Some("ns".to_owned()))
    );
}

#[rstest]
#[case::mcp_and_unknown_pass_through(
    r#"[{"type":"mcp","server_label":"s","require_approval":"never"},{"type":"foo","a":[1]},{"a":1},{"type":5}]"#,
    r#"[{"type":"mcp","server_label":"s","require_approval":"never"},{"type":"foo","a":[1]},{"a":1},{"type":5}]"#
)]
#[case::function_defaults(
    r#"[{"type":"function"}]"#,
    r#"[{"type":"function","function":{"name":"","description":"","parameters":{"type":"object"},"strict":false}}]"#
)]
#[case::function_fields(
    r#"[{"type":"function","name":"f","description":"d","parameters":{"x":1},"strict":true}]"#,
    r#"[{"type":"function","function":{"name":"f","description":"d","parameters":{"x":1,"type":"object"},"strict":true}}]"#
)]
#[case::truthy_extras_are_copied(
    r#"[{"type":"function","name":"f","cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["a"],"input_examples":[{"x":1}],"eager_input_streaming":true}]"#,
    r#"[{"type":"function","function":{"name":"f","description":"","parameters":{"type":"object"},"strict":false},"cache_control":{"type":"ephemeral"},"defer_loading":true,"allowed_callers":["a"],"input_examples":[{"x":1}],"eager_input_streaming":true}]"#
)]
#[case::falsy_extras_are_skipped_but_a_false_eager_flag_is_copied(
    r#"[{"type":"function","name":"f","cache_control":{},"defer_loading":false,"allowed_callers":[],"input_examples":[],"eager_input_streaming":false}]"#,
    r#"[{"type":"function","function":{"name":"f","description":"","parameters":{"type":"object"},"strict":false},"eager_input_streaming":false}]"#
)]
#[case::null_extras_are_skipped(
    r#"[{"type":"function","name":"f","cache_control":null,"eager_input_streaming":null}]"#,
    r#"[{"type":"function","function":{"name":"f","description":"","parameters":{"type":"object"},"strict":false}}]"#
)]
#[case::strict_is_copied_when_truthy(
    r#"[{"type":"function","name":"a","strict":"yes"},{"type":"function","name":"b","strict":0}]"#,
    r#"[{"type":"function","function":{"name":"a","description":"","parameters":{"type":"object"},"strict":"yes"}},{"type":"function","function":{"name":"b","description":"","parameters":{"type":"object"},"strict":false}}]"#
)]
#[case::custom_tool_becomes_a_function(
    r#"[{"type":"custom","name":"edit","description":"Edit"}]"#,
    r#"[{"type":"function","function":{"name":"edit","description":"Edit","parameters":{"type":"object","properties":{"content":{"type":"string","description":"The edit content following the specified format"}},"required":["content"]}}}]"#
)]
#[case::empty_list("[]", "[]")]
fn tool_forms(#[case] request: &str, #[case] expected: &str) {
    assert_eq!(chat_tools(request), parse(expected));
}

#[rstest]
#[case::context_size_and_location(
    r#"[{"type":"web_search","search_context_size":"high","user_location":{"type":"approximate","city":"Paris"}}]"#,
    r#"{"search_context_size":"high","user_location":{"type":"approximate","city":"Paris"}}"#
)]
#[case::missing_fields_are_null(
    r#"[{"type":"web_search"}]"#,
    r#"{"search_context_size":null,"user_location":null}"#
)]
#[case::preview_type(
    r#"[{"type":"web_search_preview","search_context_size":"low"}]"#,
    r#"{"search_context_size":"low","user_location":null}"#
)]
#[case::falsy_location_is_null(
    r#"[{"type":"web_search","user_location":{}}]"#,
    r#"{"search_context_size":null,"user_location":null}"#
)]
#[case::context_size_is_not_validated(
    r#"[{"type":"web_search","search_context_size":""}]"#,
    r#"{"search_context_size":"","user_location":null}"#
)]
#[case::last_search_tool_wins(
    r#"[{"type":"web_search","search_context_size":"high","user_location":{"a":1}},{"type":"web_search_preview"}]"#,
    r#"{"search_context_size":null,"user_location":null}"#
)]
fn tool_forms_web_search_options(#[case] request: &str, #[case] expected: &str) {
    let conversion = responses_tools_to_chat(&tools(request)).unwrap();
    assert_eq!(serialized(&conversion.web_search_options), parse(expected));
    assert!(conversion.chat_tools.is_empty());
}

#[test]
fn tool_forms_without_a_search_tool_have_no_options() {
    let conversion = responses_tools_to_chat(&[]).unwrap();
    assert_eq!(conversion.web_search_options, None);
    assert!(conversion.chat_tools.is_empty());
    assert!(conversion.warnings.is_empty());
}

#[rstest]
#[case::empty("[]", "[]")]
#[case::function_defaults(
    r#"[{"type":"function"}]"#,
    r#"[{"type":"function","name":"","description":"","parameters":{"type":"object"},"strict":false}]"#
)]
#[case::fields_and_non_null_extras(
    r#"[{"type":"function","function":{"name":"f","description":"d","parameters":{"x":1},"strict":true},"cache_control":{},"defer_loading":false,"allowed_callers":[],"input_examples":{},"eager_input_streaming":0}]"#,
    r#"[{"type":"function","name":"f","description":"d","parameters":{"x":1,"type":"object"},"strict":true,"cache_control":{},"defer_loading":false,"allowed_callers":[],"input_examples":{},"eager_input_streaming":0}]"#
)]
#[case::null_extras_and_falsy_strict(
    r#"[{"type":"function","function":{"name":"f","strict":0,"parameters":null},"cache_control":null,"defer_loading":null,"allowed_callers":null,"input_examples":null,"eager_input_streaming":null}]"#,
    r#"[{"type":"function","name":"f","description":"","parameters":{"type":"object"},"strict":false}]"#
)]
#[case::other_tools_pass_through(
    r#"[{"type":"mcp","server_label":"s"},{"type":"web_search","x":1},{"name":"untyped"}]"#,
    r#"[{"type":"mcp","server_label":"s"},{"type":"web_search","x":1},{"name":"untyped"}]"#
)]
fn tool_forms_back_to_responses(#[case] request: &str, #[case] expected: &str) {
    let converted = chat_tools_to_responses(&tools(request)).unwrap();
    assert_eq!(serialized(&converted), parse(expected));
}

#[test]
fn tool_forms_back_to_responses_reject_a_function_that_is_not_an_object() {
    let error =
        chat_tools_to_responses(&tools(r#"[{"type":"function","function":"f"}]"#)).unwrap_err();
    assert!(matches!(
        error,
        NamespaceToolsError::WrongType {
            field: "function",
            ..
        }
    ));
}

fn dropped(kind: &str) -> String {
    format!(
        "Dropping Responses API tool of type '{kind}': it has no Chat Completions equivalent and the target provider would reject the request."
    )
}

#[test]
fn unsupported_builtins_warn_in_order_once_per_occurrence() {
    let conversion = responses_tools_to_chat(&tools(
        r#"[{"type":"shell"},{"type":"function","name":"keep"},{"type":"computer_use"},{"type":"image_generation"},{"type":"shell"},{"type":"local_shell"},{"type":"tool_search"}]"#,
    ))
    .unwrap();
    let kinds = [
        "shell",
        "computer_use",
        "image_generation",
        "shell",
        "local_shell",
        "tool_search",
    ];
    assert_eq!(conversion.warnings, kinds.map(dropped));
    assert_eq!(conversion.chat_tools.len(), 1);
}

#[test]
fn unsupported_builtins_leave_other_types_alone() {
    let conversion = responses_tools_to_chat(&tools(
        r#"[{"type":"code_interpreter"},{"type":"file_search"},{"type":"mcp"},{"type":"computer_use_preview"},{"type":"apply_patch"}]"#,
    ))
    .unwrap();
    assert!(conversion.warnings.is_empty());
    assert_eq!(conversion.chat_tools.len(), 5);
}

fn call(id: &str, kind: &str, name: &str, arguments: Json) -> ToolCallRecord {
    ToolCallRecord {
        id: id.to_owned(),
        kind: kind.to_owned(),
        function: FunctionRecord {
            name: name.to_owned(),
            arguments,
            status: None,
            provider_specific_fields: None,
        },
        provider_specific_fields: None,
    }
}

fn choice(tool_calls: Vec<ToolCallRecord>) -> ChoiceRecord {
    ChoiceRecord {
        tool_calls,
        provider_specific_fields: Json::Null,
    }
}

fn output_items(choices: &[ChoiceRecord], request_tools: &str) -> Json {
    let outputs = tool_outputs(choices, &tools(request_tools), ApiLimits::default()).unwrap();
    serialized(&outputs.items)
}

fn string(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn mapping(text: &str) -> ProviderFields {
    let Json::Object(fields) = parse(text) else {
        panic!("fields must be an object");
    };
    ProviderFields::Mapping(fields)
}

#[test]
fn tool_outputs_restore_the_namespace_of_a_flattened_function() {
    let choices = [choice(vec![call(
        "c",
        "function",
        "ns__f",
        string(r#"{"a":1}"#),
    )])];
    assert_eq!(
        output_items(&choices, NS_FUNCTION),
        parse(
            r#"[{"name":"f","arguments":"{\"a\":1}","call_id":"c","id":"fc_c","type":"function_call","status":"completed","namespace":"ns"}]"#
        )
    );
}

#[rstest]
#[case::null(Json::Null, "")]
#[case::empty_text(string(""), "")]
#[case::text(string("raw"), "raw")]
#[case::number(parse("5"), "5")]
fn tool_outputs_arguments_are_serialized(#[case] arguments: Json, #[case] expected: &str) {
    let items = output_items(
        &[choice(vec![call("fc_c", "function", "f", arguments)])],
        "[]",
    );
    let expected_item = format!(
        r#"[{{"name":"f","arguments":"{expected}","call_id":"fc_c","id":"fc_c","type":"function_call","status":"completed"}}]"#
    );
    assert_eq!(items, parse(&expected_item));
}

#[test]
fn tool_outputs_status_comes_from_the_function() {
    let mut incomplete = call("c", "function", "f", string("{}"));
    incomplete.function.status = Some(FunctionCallStatus::Incomplete);
    let items = output_items(&[choice(vec![incomplete])], "[]");
    assert_eq!(
        items,
        parse(
            r#"[{"name":"f","arguments":"{}","call_id":"c","id":"fc_c","type":"function_call","status":"incomplete"}]"#
        )
    );
}

#[test]
fn tool_outputs_drop_an_empty_namespace_name() {
    let request = r#"[{"type":"namespace","tools":[{"type":"function","name":"f"}]}]"#;
    let items = output_items(
        &[choice(vec![call("c", "function", "__f", string("{}"))])],
        request,
    );
    assert_eq!(
        items,
        parse(
            r#"[{"name":"f","arguments":"{}","call_id":"c","id":"fc_c","type":"function_call","status":"completed"}]"#
        )
    );
}

#[rstest]
#[case::top_level("[{\"type\":\"custom\",\"name\":\"edit\"}]")]
#[case::inside_a_namespace(
    "[{\"type\":\"namespace\",\"name\":\"ns\",\"tools\":[{\"type\":\"custom\",\"name\":\"edit\"}]}]"
)]
fn tool_outputs_custom_call_unwraps_its_content(#[case] request: &str) {
    let items = output_items(
        &[choice(vec![call(
            "c",
            "function",
            "edit",
            string(r#"{"content":"patch"}"#),
        )])],
        request,
    );
    assert_eq!(
        items,
        parse(
            r#"[{"type":"custom_tool_call","call_id":"c","id":"ctc_c","name":"edit","input":"patch","status":"completed"}]"#
        )
    );
}

#[test]
fn tool_outputs_custom_call_over_the_limit_fails() {
    let limits = ApiLimits {
        max_json_bytes: 4,
        max_event_bytes: 1024,
        max_items: 1024,
    };
    let choices = [choice(vec![call(
        "c",
        "function",
        "edit",
        string(r#"{"content":"patch"}"#),
    )])];
    let request = tools(r#"[{"type":"custom","name":"edit"}]"#);
    let error = tool_outputs(&choices, &request, limits).unwrap_err();
    assert!(matches!(error, NamespaceToolsError::Custom(_)), "{error}");
}

#[test]
fn tool_outputs_cache_every_call_before_the_type_filter() {
    let choices = [
        choice(vec![
            call("a", "function", "f", string("{}")),
            call("b", "custom", "g", string("{}")),
        ]),
        choice(vec![call("c", "function", "h", string("{}"))]),
    ];
    let outputs = tool_outputs(&choices, &[], ApiLimits::default()).unwrap();
    let keys: Vec<&str> = outputs
        .cache_writes
        .iter()
        .map(|write| write.key.as_str())
        .collect();
    assert_eq!(keys, ["a", "b", "c"]);
    assert_eq!(outputs.cache_writes[1].value, choices[0].tool_calls[1]);
    assert_eq!(outputs.items.len(), 2);
}

fn with_fields(tool_level: Option<ProviderFields>, function_level: Option<ProviderFields>) -> Json {
    let mut tool = call("c", "function", "f", string("{}"));
    tool.provider_specific_fields = tool_level;
    tool.function.provider_specific_fields = function_level;
    let items = output_items(&[choice(vec![tool])], "[]");
    let Json::List(mut listed) = items else {
        panic!("items must be a list");
    };
    let Json::Object(item) = listed.remove(0) else {
        panic!("item must be an object");
    };
    item.get("provider_specific_fields")
        .cloned()
        .unwrap_or(Json::Null)
}

fn attributes(plain: Json) -> ProviderFields {
    ProviderFields::Attributes(AttributeValue::Plain(plain))
}

fn record(pairs: &str) -> ProviderFields {
    let Json::List(items) = parse(pairs) else {
        panic!("pairs must be a list");
    };
    ProviderFields::Attributes(AttributeValue::Record(items))
}

#[rstest]
#[case::tool_beats_function(Some(mapping(r#"{"a":1}"#)), Some(mapping(r#"{"b":2}"#)), r#"{"a":1}"#)]
#[case::function_only(None, Some(mapping(r#"{"b":2}"#)), r#"{"b":2}"#)]
#[case::empty_tool_mapping_falls_back(
    Some(mapping("{}")),
    Some(mapping(r#"{"b":2}"#)),
    r#"{"b":2}"#
)]
#[case::falsy_attributes_fall_back(
    Some(attributes(Json::Null)),
    Some(mapping(r#"{"b":2}"#)),
    r#"{"b":2}"#
)]
#[case::false_attributes_fall_back(
    Some(attributes(Json::Bool(false))),
    Some(mapping(r#"{"b":2}"#)),
    r#"{"b":2}"#
)]
#[case::truthy_attributes_without_a_dict_become_empty(
    Some(attributes(string("x"))),
    Some(mapping(r#"{"b":2}"#)),
    "null"
)]
#[case::record_converts(
    Some(record(r#"[["k",1],["m","v"]]"#)),
    Some(mapping(r#"{"b":2}"#)),
    r#"{"k":1,"m":"v"}"#
)]
#[case::empty_record_stops_the_search(Some(record("[]")), Some(mapping(r#"{"b":2}"#)), "null")]
#[case::function_record(None, Some(record(r#"[["k",1]]"#)), r#"{"k":1}"#)]
#[case::none_at_all(None, None, "null")]
fn tool_outputs_provider_fields(
    #[case] tool_level: Option<ProviderFields>,
    #[case] function_level: Option<ProviderFields>,
    #[case] expected: &str,
) {
    assert_eq!(with_fields(tool_level, function_level), parse(expected));
}

#[test]
fn tool_outputs_reject_a_provider_record_that_is_not_pairs() {
    let mut tool = call("c", "function", "f", string("{}"));
    tool.provider_specific_fields = Some(record("[1]"));
    let error = tool_outputs(&[choice(vec![tool])], &[], ApiLimits::default()).unwrap_err();
    assert!(matches!(error, NamespaceToolsError::Custom(_)), "{error}");
}

const SEARCH: &str = r#"{"id":"ws_c","action":{"type":"search","query":"q"},"status":"completed","type":"web_search_call"}"#;

fn search_choice(fields: &str) -> ChoiceRecord {
    ChoiceRecord {
        tool_calls: vec![],
        provider_specific_fields: parse(fields),
    }
}

#[rstest]
#[case::mapping_values(format!(r#"{{"web_search_calls":{{"any":{SEARCH}}}}}"#), r#"["c"]"#)]
#[case::sequence(format!(r#"{{"web_search_calls":[{SEARCH}]}}"#), r#"["c"]"#)]
#[case::id_without_prefix(
    r#"{"web_search_calls":[{"id":"plain","action":{"type":"open_page","url":"u"},"status":"failed","type":"web_search_call"}]}"#.to_owned(),
    r#"["plain"]"#
)]
#[case::find_in_page_action(
    r#"{"web_search_calls":[{"id":"ws_f","action":{"type":"find_in_page","pattern":"p","url":"u"},"status":"in_progress","type":"web_search_call"}]}"#.to_owned(),
    r#"["f"]"#
)]
#[case::invalid_items_are_ignored(
    format!(r#"{{"web_search_calls":[7,null,{{"id":"ws_x"}},{{"id":"ws_y","action":{{"type":"nope"}},"status":"completed","type":"web_search_call"}},{{"id":"ws_z","action":{{"type":"search"}},"status":"bogus","type":"web_search_call"}},{SEARCH}]}}"#),
    r#"["c"]"#
)]
#[case::scalar_calls_are_ignored(r#"{"web_search_calls":"x"}"#.to_owned(), "[]")]
#[case::null_calls_are_ignored(r#"{"web_search_calls":null}"#.to_owned(), "[]")]
#[case::no_calls_field(r#"{"other":1}"#.to_owned(), "[]")]
#[case::non_mapping_provider_fields_are_ignored(r#"[{"web_search_calls":[]}]"#.to_owned(), "[]")]
fn hosted_search_output_indexes_valid_calls(#[case] fields: String, #[case] keys: &str) {
    let calls = calls_by_id(&[search_choice(&fields)]);
    assert_eq!(serialized(&calls.keys().collect::<Vec<_>>()), parse(keys));
}

#[test]
fn hosted_search_output_later_entries_overwrite_earlier_ones() {
    let first = format!(r#"{{"web_search_calls":[{SEARCH}]}}"#);
    let second = r#"{"web_search_calls":{"k":{"id":"c","action":{"type":"search","query":"later"},"status":"failed","type":"web_search_call"}}}"#;
    let calls = calls_by_id(&[search_choice(&first), search_choice(second)]);
    assert_eq!(
        serialized(&calls["c"]),
        parse(
            r#"{"id":"c","action":{"type":"search","query":"later"},"status":"failed","type":"web_search_call"}"#
        )
    );
}

#[test]
fn hosted_search_output_replaces_the_matching_function_call() {
    let fields = format!(r#"{{"web_search_calls":[{SEARCH}]}}"#);
    let choices = [ChoiceRecord {
        tool_calls: vec![
            call("c", "function", "web_search", string("{}")),
            call("d", "function", "g", string("{}")),
        ],
        provider_specific_fields: parse(&fields),
    }];
    let request = r#"[{"type":"custom","name":"web_search"}]"#;
    assert_eq!(
        output_items(&choices, request),
        parse(&format!(
            r#"[{SEARCH},{{"name":"g","arguments":"{{}}","call_id":"d","id":"fc_d","type":"function_call","status":"completed"}}]"#
        ))
    );
}
