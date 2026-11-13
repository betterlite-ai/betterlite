#![cfg(test)]

use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use super::output_format::{OutputFormatError, output_format_to_openai, strict_object_schema};
use super::request::{translate_output_format, translate_tool_choice, translate_tools};
use super::tool_choice::{ToolChoiceError, tool_choice_to_openai};
use super::tools::{ToolsError, ToolsTranslation, tools_to_openai};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("fixture literal is a JSON object")
}

fn tools_fixture(text: &str) -> Vec<Json> {
    let Json::List(tools) = json(text) else {
        panic!("fixture {text} is a tool list");
    };
    tools
}

fn translate(text: &str, model: Option<&str>) -> Result<ToolsTranslation, ToolsError> {
    tools_to_openai(&tools_fixture(text), model)
}

fn translated(text: &str, model: Option<&str>) -> Json {
    Json::List(translate(text, model).expect("the tools translate").tools)
}

fn name_mapping(text: &str) -> Json {
    let translation = translate(text, None).expect("the tools translate");
    let mapping: Object = translation
        .name_mapping
        .into_iter()
        .map(|(name, original)| (name, Json::Text(original)))
        .collect();
    Json::Object(mapping)
}

fn choice(text: &str) -> Json {
    tool_choice_to_openai(&json(text)).expect("the choice translates")
}

fn format(text: &str) -> Result<Option<Json>, OutputFormatError> {
    output_format_to_openai(&json(text))
}

fn format_answer(text: &str) -> Json {
    format(text)
        .expect("the format translates")
        .expect("a response format")
}

#[rstest]
#[case::any(r#"{"type": "any"}"#, r#""required""#)]
#[case::auto(r#"{"type": "auto"}"#, r#""auto""#)]
#[case::none(r#"{"type": "none"}"#, r#""none""#)]
#[case::tool(
    r#"{"type": "tool", "name": "lookup"}"#,
    r#"{"type": "function", "function": {"name": "lookup"}}"#
)]
#[case::tool_without_a_name(
    r#"{"type": "tool"}"#,
    r#"{"type": "function", "function": {"name": ""}}"#
)]
#[case::tool_with_a_null_name(
    r#"{"type": "tool", "name": null}"#,
    r#"{"type": "function", "function": {"name": ""}}"#
)]
#[case::tool_name_at_the_limit(
    r#"{"type": "tool", "name": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
    r#"{"type": "function", "function": {"name": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#
)]
#[case::tool_with_a_long_name(
    r#"{"type": "tool", "name": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
    r#"{"type": "function", "function": {"name": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa_635361c4"}}"#
)]
#[case::tool_ignores_parallel_flag(
    r#"{"type": "tool", "name": "lookup", "disable_parallel_tool_use": true}"#,
    r#"{"type": "function", "function": {"name": "lookup"}}"#
)]
fn tool_choice_variants_map_onto_the_chat_shapes(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(choice(input), json(expected));
}

#[rstest]
#[case::missing_type("{}", ToolChoiceError::MissingType)]
#[case::missing_type_with_name(r#"{"name": "lookup"}"#, ToolChoiceError::MissingType)]
#[case::unknown_type(r#"{"type": "bogus"}"#, ToolChoiceError::Unsupported)]
#[case::null_type(r#"{"type": null}"#, ToolChoiceError::Unsupported)]
#[case::numeric_type(r#"{"type": 7}"#, ToolChoiceError::Unsupported)]
#[case::list_type(r#"{"type": ["any"]}"#, ToolChoiceError::Unsupported)]
#[case::numeric_name(r#"{"type": "tool", "name": 7}"#, ToolChoiceError::Name)]
#[case::list_name(r#"{"type": "tool", "name": ["lookup"]}"#, ToolChoiceError::Name)]
fn tool_choice_failures_are_typed(#[case] input: &str, #[case] expected: ToolChoiceError) {
    assert_eq!(
        tool_choice_to_openai(&json(input)),
        Err(expected),
        "{input}"
    );
}

#[rstest]
#[case::number("7")]
#[case::null("null")]
#[case::list("[1]")]
#[case::text(r#""any""#)]
fn tool_choice_shape_failures_are_typed(#[case] input: &str) {
    assert!(matches!(
        tool_choice_to_openai(&json(input)),
        Err(ToolChoiceError::Shape)
    ));
}

#[rstest]
#[case::web_search(r#"[{"type": "web_search_20260209", "name": "web_search", "max_uses": 3}]"#)]
#[case::bash(r#"[{"type": "bash_20250124", "name": "bash"}]"#)]
#[case::text_editor(r#"[{"type": "text_editor_20250728", "name": "editor"}]"#)]
#[case::code_execution(r#"[{"type": "code_execution_20250825", "name": "code"}]"#)]
#[case::web_fetch(r#"[{"type": "web_fetch_20250910", "name": "fetch"}]"#)]
#[case::memory(r#"[{"type": "memory_20250818", "name": "memory"}]"#)]
#[case::tool_search(r#"[{"type": "tool_search_tool_regex_20251119", "name": "search"}]"#)]
#[case::openai_function(r#"[{"type": "function", "function": {"name": "f"}}]"#)]
#[case::provider_native(r#"[{"googleSearch": {}}]"#)]
fn hosted_and_native_tools_pass_through_unchanged(#[case] input: &str) {
    assert_eq!(translated(input, None), json(input));
}

#[test]
fn a_hosted_tool_keeps_its_cache_control_unrewritten() {
    let tool =
        r#"[{"type": "bash_20250124", "name": "bash", "cache_control": {"type": "ephemeral"}}]"#;
    assert_eq!(
        translated(tool, Some("anthropic/claude-3-opus")),
        json(tool)
    );
}

#[test]
fn declaration_defaults_build_a_function_tool() {
    let input = r#"[{"type": "custom", "name": "lookup", "input_schema": {"type": "object"}, "description": "find", "strict": false}]"#;
    assert_eq!(
        translated(input, None),
        json(
            r#"[{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object"}, "description": "find", "strict": false}}]"#
        )
    );
}

#[test]
fn a_custom_type_never_overwrites_the_schema_type() {
    let input = r#"[{"type": "custom", "name": "lookup", "input_schema": {"type": "object"}}]"#;
    assert_eq!(
        translated(input, None),
        json(
            r#"[{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object"}}}]"#
        )
    );
}

#[test]
fn a_type_without_a_schema_creates_no_parameters() {
    assert_eq!(
        translated(r#"[{"type": "custom", "name": "lookup"}]"#, None),
        json(r#"[{"type": "function", "function": {"name": "lookup"}}]"#)
    );
}

#[test]
fn a_description_is_copied_whatever_its_value() {
    assert_eq!(
        translated(
            r#"[{"name": "a", "description": "find"}, {"name": "b", "description": null}]"#,
            None
        ),
        json(
            r#"[{"type": "function", "function": {"name": "a", "description": "find"}}, {"type": "function", "function": {"name": "b", "description": null}}]"#
        )
    );
}

#[rstest]
#[case::missing("[{}]", "litellm_unnamed_tool_0")]
#[case::null(r#"[{"name": null}]"#, "litellm_unnamed_tool_0")]
#[case::empty(r#"[{"name": ""}]"#, "litellm_unnamed_tool_0")]
#[case::whitespace(r#"[{"name": "   "}]"#, "litellm_unnamed_tool_0")]
#[case::second_slot(r#"[{"name": "a"}, {"name": " "}]"#, "litellm_unnamed_tool_1")]
#[case::number(r#"[{"name": 7}]"#, "7")]
fn unnamed_tools_take_the_index_or_the_number_as_text(#[case] input: &str, #[case] expected: &str) {
    let expected_tools =
        format!(r#"[{{"type": "function", "function": {{"name": "{expected}"}}}}]"#);
    assert_eq!(translated(input, None), json(&expected_tools));
}

#[rstest]
#[case::null(r#"[{"type": null, "name": "a"}]"#)]
#[case::number(r#"[{"type": 7, "name": "a"}]"#)]
#[case::list(r#"[{"type": ["custom"], "name": "a"}]"#)]
fn a_non_text_type_is_an_attribute_failure(#[case] input: &str) {
    assert!(matches!(
        translate(input, None),
        Err(ToolsError::TypeNotText { index: 0 })
    ));
}

#[rstest]
#[case::number("7")]
#[case::text(r#""tool""#)]
#[case::object(r#"{"name": "a"}"#)]
fn a_tool_that_is_not_an_object_fails(#[case] tool: &str) {
    let input = format!("[{tool}]");
    assert!(matches!(
        translate(&input, None),
        Err(ToolsError::NotAnObject { index: 0 })
    ));
}

#[rstest]
#[case::list(r#"[{"name": ["a"]}]"#)]
#[case::object(r#"[{"name": {"a": 1}}]"#)]
fn a_name_of_another_shape_fails(#[case] input: &str) {
    assert!(matches!(
        translate(input, None),
        Err(ToolsError::NameNotText { index: 0 })
    ));
}

#[test]
fn a_missing_or_null_schema_is_omitted() {
    assert_eq!(
        translated(r#"[{"name": "a"}]"#, None),
        json(r#"[{"type": "function", "function": {"name": "a"}}]"#)
    );
    assert_eq!(
        translated(r#"[{"name": "a", "input_schema": null}]"#, None),
        json(r#"[{"type": "function", "function": {"name": "a"}}]"#)
    );
}

#[test]
fn a_non_object_schema_fails_unless_an_extra_field_replaces_it() {
    assert!(matches!(
        translate(r#"[{"name": "a", "input_schema": [1]}]"#, None),
        Err(ToolsError::SchemaNotObject { index: 0 })
    ));
    assert_eq!(
        translated(r#"[{"name": "a", "input_schema": null, "x": 1}]"#, None),
        json(r#"[{"type": "function", "function": {"name": "a", "parameters": {"x": 1}}}]"#)
    );
}

#[test]
fn extra_fields_merge_into_the_source_schema() {
    let input = r#"[{"name": "lookup", "input_schema": {"type": "object"}, "type": "custom", "display_width_px": 800}]"#;
    assert_eq!(
        translated(input, None),
        json(
            r#"[{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object", "display_width_px": 800}}}]"#
        )
    );
}

#[test]
fn an_extra_field_creates_the_parameters_it_merges_into() {
    let input = r#"[{"name": "lookup", "display_width_px": 800}]"#;
    assert_eq!(
        translated(input, None),
        json(
            r#"[{"type": "function", "function": {"name": "lookup", "parameters": {"display_width_px": 800}}}]"#
        )
    );
}

#[test]
fn an_extra_field_overwrites_the_schema_key_of_the_same_name() {
    let input =
        r#"[{"name": "lookup", "input_schema": {"width": 1, "type": "object"}, "width": 2}]"#;
    assert_eq!(
        translated(input, None),
        json(
            r#"[{"type": "function", "function": {"name": "lookup", "parameters": {"width": 2, "type": "object"}}}]"#
        )
    );
}

#[rstest]
#[case::bool_false("false", false)]
#[case::bool_true("true", true)]
#[case::string("false", true)]
#[case::zero("0", false)]
#[case::null("null", false)]
#[case::object(r#"{"a": 1}"#, true)]
fn strict_is_python_truthiness(#[case] strict: &str, #[case] expected: bool) {
    let input = format!(r#"[{{"name": "a", "strict": {strict}}}]"#);
    let expected_tools =
        format!(r#"[{{"type": "function", "function": {{"name": "a", "strict": {expected}}}}}]"#);
    assert_eq!(translated(&input, None), json(&expected_tools));
}

#[rstest]
#[case::top_level_true(r#"{"name": "a", "eager_input_streaming": true}"#)]
#[case::nested_true(r#"{"name": "a", "function": {"eager_input_streaming": true}}"#)]
fn eager_input_streaming_stays_at_the_tool_level(#[case] declaration: &str) {
    let input = format!("[{declaration}]");
    let translated_tools = translated(&input, None);
    let Json::List(tools) = translated_tools else {
        panic!("a tool list answers");
    };
    let Some(converted) = tools.first().and_then(Json::as_object) else {
        panic!("one tool answers");
    };
    assert_eq!(
        converted.get("eager_input_streaming"),
        Some(&Json::Bool(true))
    );
    let Some(Json::Object(function)) = converted.get("function") else {
        panic!("a function chunk answers");
    };
    assert_eq!(function.get("eager_input_streaming"), None);
}

#[test]
fn a_top_level_false_flag_wins_over_the_nested_one() {
    let input = r#"[{"name": "a", "eager_input_streaming": false, "function": {"eager_input_streaming": true}}]"#;
    let Json::List(tools) = translated(input, None) else {
        panic!("a tool list answers");
    };
    let Some(tool) = tools.first().and_then(Json::as_object) else {
        panic!("one tool answers");
    };
    assert_eq!(tool.get("eager_input_streaming"), Some(&Json::Bool(false)));
}

#[test]
fn a_non_boolean_eager_flag_is_an_unsupported_parameter() {
    let error = translate(r#"[{"name": "a", "eager_input_streaming": "true"}]"#, None)
        .expect_err("the flag is rejected");
    assert!(matches!(
        error,
        ToolsError::EagerInput(FeatureFlagsError::EagerInputStreaming)
    ));
}

#[rstest]
#[case::claude(Some("anthropic/claude-3-opus"), Some(r#"{"type": "ephemeral"}"#))]
#[case::bare_claude(Some("claude-3-opus"), Some(r#"{"type": "ephemeral"}"#))]
#[case::gemini(Some("gemini/gemini-2.0-flash"), Some(r#"{"type": "ephemeral"}"#))]
#[case::bedrock_arn(
    Some("bedrock/arn:aws:bedrock:us-east-1:1:application-inference-profile/x"),
    Some(r#"{"type": "ephemeral"}"#)
)]
#[case::openai(Some("gpt-4o"), None)]
#[case::absent(None, None)]
#[case::empty(Some(""), None)]
fn cache_control_follows_the_target_model(
    #[case] model: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let input = r#"[{"name": "a", "cache_control": {"type": "ephemeral"}}]"#;
    let Json::List(tools) = translated(input, model) else {
        panic!("a tool list answers");
    };
    let Some(tool) = tools.first().and_then(Json::as_object) else {
        panic!("one tool answers");
    };
    assert_eq!(tool.get("cache_control").cloned(), expected.map(json));
}

#[rstest]
#[case::empty(r#"{"name": "a", "cache_control": {}}"#)]
#[case::null(r#"{"name": "a", "cache_control": null}"#)]
fn a_falsey_cache_control_is_not_copied(#[case] declaration: &str) {
    let input = format!("[{declaration}]");
    let Json::List(tools) = translated(&input, Some("claude-3-opus")) else {
        panic!("a tool list answers");
    };
    let Some(converted) = tools.first().and_then(Json::as_object) else {
        panic!("one tool answers");
    };
    assert_eq!(converted.get("cache_control"), None);
}

#[test]
fn only_truncated_names_enter_the_mapping() {
    let input = format!(
        r#"[{{"name": "{}"}}, {{"name": "{}"}}, {{"name": "a"}}]"#,
        "b".repeat(65),
        "c".repeat(64)
    );
    let truncated = format!("{}_74b128f3", "b".repeat(55));
    let expected = format!(r#"{{"{truncated}": "{}"}}"#, "b".repeat(65));
    assert_eq!(name_mapping(&input), json(&expected));
}

#[test]
fn a_short_name_or_passed_through_tool_leaves_the_mapping_empty() {
    assert_eq!(
        name_mapping(r#"[{"name": "lookup"}, {"type": "function", "function": {"name": "a"}}]"#),
        json("{}")
    );
}

#[test]
fn two_long_names_sharing_a_prefix_stay_apart() {
    let input = format!(
        r#"[{{"name": "{prefix}a"}}, {{"name": "{prefix}b"}}]"#,
        prefix = "a".repeat(64)
    );
    let answer = name_mapping(&input);
    let Json::Object(entries) = answer else {
        panic!("a mapping object answers");
    };
    assert_eq!(entries.len(), 2);
    assert!(!entries.contains_key(&"a".repeat(65)));
}

#[test]
fn an_empty_tool_list_translates_to_an_empty_answer() {
    let translation = translate("[]", None).expect("an empty list translates");
    assert!(translation.tools.is_empty());
    assert!(translation.name_mapping.is_empty());
}

#[test]
fn strict_output_schema_rewrites_every_object() {
    let input = r#"{"type": "json_schema", "schema": {"type": "object", "properties": {"x": {"type": "string"}}, "required": [], "additionalProperties": true}}"#;
    assert_eq!(
        format_answer(input),
        json(
            r#"{"type": "json_schema", "json_schema": {"name": "structured_output", "strict": true, "schema": {"type": "object", "properties": {"x": {"type": "string"}}, "required": ["x"], "additionalProperties": false}}}"#
        )
    );
}

#[test]
fn required_lists_the_properties_in_their_order() {
    let input = r#"{"type": "json_schema", "schema": {"type": "object", "properties": {"z": {"type": "string"}, "a": {"type": "number"}}, "required": ["a"]}}"#;
    let answer = format_answer(input);
    let Some(Json::Object(response)) = answer
        .as_object()
        .and_then(|fields| fields.get("json_schema"))
    else {
        panic!("a json_schema answers");
    };
    let Some(Json::Object(schema)) = response.get("schema") else {
        panic!("a schema answers");
    };
    assert_eq!(schema.get("required"), Some(&json(r#"["z", "a"]"#)));
}

#[rstest]
#[case::items("items")]
#[case::any_of("anyOf")]
#[case::one_of("oneOf")]
#[case::all_of("allOf")]
fn nested_schemas_in_arrays_are_rewritten(#[case] key: &str) {
    let input = format!(
        r#"{{"type": "json_schema", "schema": {{"{key}": [{{"type": "object", "properties": {{"b": {{"type": "string"}}}}}}]}}}}"#
    );
    let answer = format_answer(&input);
    let Some(Json::Object(response)) = answer
        .as_object()
        .and_then(|fields| fields.get("json_schema"))
    else {
        panic!("a json_schema answers");
    };
    let Some(Json::Object(schema)) = response.get("schema") else {
        panic!("a schema answers");
    };
    let Some(Json::List(entries)) = schema.get(key) else {
        panic!("a schema list answers");
    };
    let Some(Json::Object(entry)) = entries.first() else {
        panic!("one schema answers");
    };
    assert_eq!(entry.get("additionalProperties"), Some(&Json::Bool(false)));
    assert_eq!(entry.get("required"), Some(&json(r#"["b"]"#)));
}

#[rstest]
#[case::defs("$defs")]
#[case::definitions("definitions")]
fn nested_schemas_in_definitions_are_rewritten(#[case] key: &str) {
    let input = format!(
        r#"{{"type": "json_schema", "schema": {{"{key}": {{"named": {{"type": "object", "properties": {{"e": {{"type": "string"}}}}}}}}}}}}"#
    );
    let expected = format!(
        r#"{{"type": "json_schema", "json_schema": {{"name": "structured_output", "strict": true, "schema": {{"{key}": {{"named": {{"type": "object", "properties": {{"e": {{"type": "string"}}}}, "required": ["e"], "additionalProperties": false}}}}}}}}}}"#
    );
    assert_eq!(format_answer(&input), json(&expected));
}

#[test]
fn an_object_without_properties_is_unchanged() {
    assert_eq!(
        format_answer(r#"{"type": "json_schema", "schema": {"type": "object"}}"#),
        json(
            r#"{"type": "json_schema", "json_schema": {"name": "structured_output", "strict": true, "schema": {"type": "object"}}}"#
        )
    );
}

#[test]
fn a_property_that_is_not_an_object_is_left_alone() {
    let input = r#"{"type": "json_schema", "schema": {"type": "object", "properties": {"a": 7}}}"#;
    let answer = format_answer(input);
    let Some(Json::Object(response)) = answer
        .as_object()
        .and_then(|fields| fields.get("json_schema"))
    else {
        panic!("a json_schema answers");
    };
    let Some(Json::Object(schema)) = response.get("schema") else {
        panic!("a schema answers");
    };
    assert_eq!(schema.get("required"), Some(&json(r#"["a"]"#)));
}

#[test]
fn a_non_object_schema_passes_through() {
    assert_eq!(
        format_answer(r#"{"type": "json_schema", "schema": 7}"#),
        json(
            r#"{"type": "json_schema", "json_schema": {"name": "structured_output", "strict": true, "schema": 7}}"#
        )
    );
}

#[rstest]
#[case::missing_schema(r#"{"type": "json_schema"}"#)]
#[case::null_schema(r#"{"type": "json_schema", "schema": null}"#)]
#[case::empty_schema(r#"{"type": "json_schema", "schema": {}}"#)]
#[case::empty_list_schema(r#"{"type": "json_schema", "schema": []}"#)]
#[case::zero_schema(r#"{"type": "json_schema", "schema": 0}"#)]
#[case::text_type(r#"{"type": "text", "schema": {"type": "object"}}"#)]
#[case::missing_type(r#"{"schema": {"type": "object"}}"#)]
#[case::null_type(r#"{"type": null, "schema": {"type": "object"}}"#)]
#[case::scalar("7")]
#[case::list("[]")]
#[case::null("null")]
fn a_format_that_is_not_a_json_schema_yields_nothing(#[case] input: &str) {
    assert_eq!(format(input), Ok(None));
}

#[test]
fn the_callers_schema_is_not_rewritten() {
    let output_format = Json::Object(object(
        r#"{"type": "json_schema", "schema": {"type": "object", "properties": {"x": {"type": "string"}}}}"#,
    ));
    assert!(
        output_format_to_openai(&output_format)
            .expect("the format translates")
            .is_some()
    );
    assert_eq!(
        output_format,
        json(
            r#"{"type": "json_schema", "schema": {"type": "object", "properties": {"x": {"type": "string"}}}}"#
        )
    );
}

#[test]
fn the_schema_walker_rewrites_in_place() {
    let mut schema = json(r#"{"type": "object", "properties": {"x": {"type": "string"}}}"#);
    strict_object_schema(&mut schema).expect("the schema rewrites");
    assert_eq!(
        schema,
        json(
            r#"{"type": "object", "properties": {"x": {"type": "string"}}, "required": ["x"], "additionalProperties": false}"#
        )
    );
}

#[test]
fn a_properties_value_that_is_not_an_object_fails() {
    let input = r#"{"type": "json_schema", "schema": {"type": "object", "properties": [1]}}"#;
    assert!(matches!(format(input), Err(OutputFormatError::Properties)));
}

#[rstest]
#[case::number("7")]
#[case::boolean("true")]
fn a_union_value_that_is_not_a_list_fails(#[case] value: &str) {
    let input = format!(r#"{{"type": "json_schema", "schema": {{"anyOf": {value}}}}}"#);
    assert!(matches!(
        format(&input),
        Err(OutputFormatError::SubschemaList { key: "anyOf" })
    ));
}

#[rstest]
#[case::list("[1]")]
#[case::number("7")]
fn a_definitions_value_that_is_not_an_object_fails(#[case] value: &str) {
    let input = format!(r#"{{"type": "json_schema", "schema": {{"$defs": {value}}}}}"#);
    assert!(matches!(
        format(&input),
        Err(OutputFormatError::Definitions { key: "$defs" })
    ));
}

#[test]
fn a_scalar_items_value_is_ignored() {
    assert_eq!(
        format_answer(r#"{"type": "json_schema", "schema": {"items": 7}}"#),
        json(
            r#"{"type": "json_schema", "json_schema": {"name": "structured_output", "strict": true, "schema": {"items": 7}}}"#
        )
    );
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"tool_choice": null}"#)]
#[case::empty_object(r#"{"tool_choice": {}}"#)]
#[case::empty_string(r#"{"tool_choice": ""}"#)]
#[case::zero(r#"{"tool_choice": 0}"#)]
#[case::bool_false(r#"{"tool_choice": false}"#)]
#[case::empty_list(r#"{"tool_choice": []}"#)]
fn a_falsey_tool_choice_leaves_the_kwargs_alone(#[case] request: &str) {
    let mut kwargs = object(r#"{"model": "gpt-4o"}"#);
    translate_tool_choice(&object(request), &mut kwargs).expect("the choice is skipped");
    assert_eq!(kwargs, object(r#"{"model": "gpt-4o"}"#));
}

#[test]
fn a_truthy_tool_choice_is_written_into_the_kwargs() {
    let mut kwargs = object(r#"{"model": "gpt-4o"}"#);
    translate_tool_choice(&object(r#"{"tool_choice": {"type": "any"}}"#), &mut kwargs)
        .expect("the choice translates");
    assert_eq!(
        kwargs,
        object(r#"{"model": "gpt-4o", "tool_choice": "required"}"#)
    );
}

#[test]
fn a_tool_choice_failure_reaches_the_caller() {
    let mut kwargs = Object::new();
    assert_eq!(
        translate_tool_choice(
            &object(r#"{"tool_choice": {"type": "bogus"}}"#),
            &mut kwargs
        ),
        Err(ToolChoiceError::Unsupported)
    );
}

#[rstest]
#[case::absent("{}")]
#[case::empty_list(r#"{"tools": []}"#)]
fn absent_or_empty_tools_leave_the_kwargs_alone(#[case] request: &str) {
    let mut kwargs = object(r#"{"model": "gpt-4o", "web_search_options": {"kept": true}}"#);
    let mapping = translate_tools(&object(request), &mut kwargs).expect("the tools are skipped");
    assert!(mapping.is_empty());
    assert_eq!(
        kwargs,
        object(r#"{"model": "gpt-4o", "web_search_options": {"kept": true}}"#)
    );
}

#[test]
fn web_search_is_extracted_and_the_rest_translates() {
    let request = r#"{"tools": [{"type": "web_search_20260209", "name": "web_search"}, {"name": "lookup", "input_schema": {"type": "object"}}]}"#;
    let mut kwargs = object(r#"{"model": "gpt-4o"}"#);
    let mapping = translate_tools(&object(request), &mut kwargs).expect("the tools translate");
    assert!(mapping.is_empty());
    assert_eq!(
        kwargs,
        object(
            r#"{"model": "gpt-4o", "web_search_options": {}, "tools": [{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object"}}}]}"#
        )
    );
}

#[test]
fn web_search_alone_leaves_the_tools_key_absent() {
    let request =
        r#"{"tools": [{"type": "web_search_20260209", "name": "web_search", "max_uses": 3}]}"#;
    let mut kwargs = object(r#"{"model": "gpt-4o"}"#);
    let mapping = translate_tools(&object(request), &mut kwargs).expect("the tools translate");
    assert!(mapping.is_empty());
    assert_eq!(
        kwargs,
        object(r#"{"model": "gpt-4o", "web_search_options": {}}"#)
    );
}

#[test]
fn the_legacy_web_search_name_is_extracted_without_a_schema() {
    let request = r#"{"tools": [{"name": "web_search", "max_uses": 2}]}"#;
    let mut kwargs = object(r#"{"model": "gpt-4o"}"#);
    translate_tools(&object(request), &mut kwargs).expect("the tools translate");
    assert_eq!(
        kwargs,
        object(r#"{"model": "gpt-4o", "web_search_options": {}}"#)
    );
}

#[test]
fn the_legacy_web_search_name_with_a_schema_is_a_function() {
    let request = r#"{"tools": [{"name": "web_search", "input_schema": {"type": "object"}}]}"#;
    let mut kwargs = object(r#"{"model": "gpt-4o"}"#);
    translate_tools(&object(request), &mut kwargs).expect("the tools translate");
    assert_eq!(
        kwargs,
        object(
            r#"{"model": "gpt-4o", "tools": [{"type": "function", "function": {"name": "web_search", "parameters": {"type": "object"}}}]}"#
        )
    );
}

#[test]
fn the_long_name_map_survives_the_request_wrapper() {
    let long = "a".repeat(65);
    let request = format!(r#"{{"tools": [{{"name": "{long}"}}]}}"#);
    let mut kwargs = object(r#"{"model": "gpt-4o"}"#);
    let mapping = translate_tools(&object(&request), &mut kwargs).expect("the tools translate");
    assert_eq!(mapping.len(), 1);
    assert_eq!(mapping.values().next(), Some(&long));
}

#[test]
fn a_tools_value_that_is_not_a_list_fails() {
    let mut kwargs = Object::new();
    assert!(matches!(
        translate_tools(&object(r#"{"tools": {"name": "a"}}"#), &mut kwargs),
        Err(ToolsError::NotAList)
    ));
}

#[test]
fn a_non_object_tool_fails_inside_the_wrapper() {
    let mut kwargs = object(r#"{"model": "gpt-4o"}"#);
    assert!(matches!(
        translate_tools(&object(r#"{"tools": [7]}"#), &mut kwargs),
        Err(ToolsError::NotAnObject { index: 0 })
    ));
}

#[rstest]
#[case::number(r#"{"model": 7}"#)]
#[case::list(r#"{"model": ["gpt-4o"]}"#)]
fn a_model_that_is_not_a_string_fails(#[case] kwargs: &str) {
    let request = object(r#"{"tools": [{"name": "a"}]}"#);
    assert!(matches!(
        translate_tools(&request, &mut object(kwargs)),
        Err(ToolsError::ModelNotText)
    ));
}

#[test]
fn an_empty_model_skips_cache_control() {
    let request = object(r#"{"tools": [{"name": "a", "cache_control": {"type": "ephemeral"}}]}"#);
    let mut kwargs = object(r#"{"model": ""}"#);
    translate_tools(&request, &mut kwargs).expect("the tools translate");
    let Some(Json::List(tools)) = kwargs.get("tools") else {
        panic!("a tool list answers");
    };
    let Some(Json::Object(tool)) = tools.first() else {
        panic!("one tool answers");
    };
    assert_eq!(tool.get("cache_control"), None);
}

#[test]
fn output_format_wins_over_the_nested_one() {
    let request = r#"{"output_format": {"type": "json_schema", "schema": {"type": "object", "properties": {"top": {"type": "string"}}}}, "output_config": {"format": {"type": "json_schema", "schema": {"type": "object", "properties": {"nested": {"type": "string"}}}}}}"#;
    let mut kwargs = Object::new();
    translate_output_format(&object(request), &mut kwargs).expect("the format translates");
    assert_eq!(
        kwargs,
        object(
            r#"{"response_format": {"type": "json_schema", "json_schema": {"name": "structured_output", "strict": true, "schema": {"type": "object", "properties": {"top": {"type": "string"}}, "required": ["top"], "additionalProperties": false}}}}"#
        )
    );
}

#[rstest]
#[case::null(r#"{"output_format": null, "output_config": {"format": {"type": "json_schema", "schema": {"type": "object"}}}}"#)]
#[case::empty_object(r#"{"output_format": {}, "output_config": {"format": {"type": "json_schema", "schema": {"type": "object"}}}}"#)]
#[case::empty_string(r#"{"output_format": "", "output_config": {"format": {"type": "json_schema", "schema": {"type": "object"}}}}"#)]
fn a_falsey_output_format_falls_back_to_the_nested_one(#[case] request: &str) {
    let mut kwargs = Object::new();
    translate_output_format(&object(request), &mut kwargs).expect("the format translates");
    assert!(kwargs.contains_key("response_format"));
}

#[test]
fn a_truthy_invalid_output_format_does_not_fall_back() {
    let request = r#"{"output_format": {"type": "text"}, "output_config": {"format": {"type": "json_schema", "schema": {"type": "object"}}}}"#;
    let mut kwargs = Object::new();
    translate_output_format(&object(request), &mut kwargs).expect("the format is skipped");
    assert!(!kwargs.contains_key("response_format"));
}

#[rstest]
#[case::absent(r#"{"output_config": {}}"#)]
#[case::empty_nested(r#"{"output_config": {"format": {}}}"#)]
#[case::null_nested(r#"{"output_config": {"format": null}}"#)]
#[case::text_config(r#"{"output_config": "format"}"#)]
fn a_format_that_cannot_be_read_leaves_response_format_alone(#[case] request: &str) {
    let mut kwargs = object(r#"{"response_format": {"kept": true}}"#);
    translate_output_format(&object(request), &mut kwargs).expect("the format is skipped");
    assert_eq!(kwargs, object(r#"{"response_format": {"kept": true}}"#));
}

#[test]
fn a_schema_failure_reaches_the_caller() {
    let request = r#"{"output_format": {"type": "json_schema", "schema": {"type": "object", "properties": 7}}}"#;
    let mut kwargs = Object::new();
    assert!(matches!(
        translate_output_format(&object(request), &mut kwargs),
        Err(OutputFormatError::Properties)
    ));
}
