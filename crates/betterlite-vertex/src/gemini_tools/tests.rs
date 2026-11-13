#![cfg(test)]

use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::assembly::{MappedTools, map_tools};
use super::cleanup::remove_unsupported_fields;
use super::error::GeminiToolsError;
use super::options::{drop_mixed_search_tools, map_service_tier, map_tool_choice};
use super::records::{CallingMode, FunctionCallingConfig, FunctionDeclaration, Tool, ToolConfig};
use super::specials::{extract_maps, transform_computer_use, transform_type};
use super::tool_lookup::{tool_value, underscore_name};

fn parse<T>(source: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(source).unwrap()
}

fn as_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    let text = serde_json::to_string(value).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn tools(source: &str) -> Vec<Object> {
    parse(source)
}

fn mapped(tools: &[Object]) -> MappedTools {
    map_tools(tools, false, None, &Settings::default()).unwrap()
}

#[test]
fn native_tools_keep_the_source_order_and_one_record_per_kind() {
    let input: Vec<Object> = parse(
        r#"[
            {"computerUse": {}},
            {"googleMaps": {"languageCode": "en"}},
            {"urlContext": {}},
            {"code_execution": {"code": "config"}},
            {"enterpriseWebSearch": {}},
            {"google_search_retrieval": {"dynamicRetrievalConfig": {"mode": "MODE_DYNAMIC"}}},
            {"google_search": {"searchType": "SEARCH_TYPE_MEDIA"}}
        ]"#,
    );
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(
            r#"[
                {"googleSearch": {"searchType": "SEARCH_TYPE_MEDIA"}},
                {"googleSearchRetrieval": {"dynamicRetrievalConfig": {"mode": "MODE_DYNAMIC"}}},
                {"enterpriseWebSearch": {}},
                {"code_execution": {"code": "config"}},
                {"url_context": {}},
                {"googleMaps": {}},
                {"computerUse": {}}
            ]"#,
        )
    );
    assert_eq!(mapped.warnings, 0);
}

#[test]
fn function_declarations_are_grouped_first_and_keep_the_specials_that_coexist() {
    let input: Vec<Object> = parse(
        r#"[
            {"computer_use": {"environment": "browser"}},
            {"google_maps": {}},
            {"codeExecution": {}},
            {"type": "function", "function": {"name": "first", "description": "d", "parameters": {"type": "object"}}},
            {"name": "second"}
        ]"#,
    );
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(
            r#"[
                {"function_declarations": [
                    {"name": "first", "description": "d", "parameters": {"type": "object"}},
                    {"name": "second"}
                ]},
                {"code_execution": {}},
                {"googleMaps": {}},
                {"computerUse": {"environment": "ENVIRONMENT_BROWSER"}}
            ]"#,
        )
    );
}

#[test]
fn a_repeated_special_tool_keeps_the_last_one() {
    let input: Vec<Object> = parse(
        r#"[
            {"googleSearch": {"first": true}},
            {"googleSearch": {"second": true}},
            {"computerUse": {"environment": "browser"}},
            {"computerUse": {"environment": "unspecified"}}
        ]"#,
    );
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(
            r#"[
                {"googleSearch": {"second": true}},
                {"computerUse": {"environment": "ENVIRONMENT_UNSPECIFIED"}}
            ]"#,
        )
    );
}

#[rstest]
#[case::dropped(
    false,
    r#"[
        {"function_declarations": [{"name": "f"}]},
        {"code_execution": {}},
        {"googleMaps": {}},
        {"computerUse": {}}
    ]"#
)]
#[case::server_side_opt_in(
    true,
    r#"[
        {"function_declarations": [{"name": "f"}]},
        {"googleSearch": {}},
        {"googleSearchRetrieval": {}},
        {"enterpriseWebSearch": {}},
        {"code_execution": {}},
        {"url_context": {}},
        {"googleMaps": {}},
        {"computerUse": {}}
    ]"#
)]
fn search_tools_mixed_with_functions_follow_the_server_side_flag(
    #[case] include_server_side: bool,
    #[case] expected: &str,
) {
    let input: Vec<Object> = parse(
        r#"[
            {"type": "function", "function": {"name": "f"}},
            {"googleSearch": {}},
            {"googleSearchRetrieval": {}},
            {"enterpriseWebSearch": {}},
            {"url_context": {}},
            {"code_execution": {}},
            {"googleMaps": {}},
            {"computerUse": {}}
        ]"#,
    );
    let mapped = map_tools(&input, include_server_side, None, &Settings::default()).unwrap();
    assert_eq!(as_json(&mapped.tools), parse::<Json>(expected));
    assert_eq!(mapped.warnings, usize::from(!include_server_side));
}

#[rstest]
#[case::no_functions(r#"{"googleSearch": {}}, {"code_execution": {}}"#)]
#[case::empty_declarations(r#"{"function_declarations": []}, {"googleSearch": {}}"#)]
#[case::no_search(r#"{"function_declarations": [{"name": "f"}]}, {"code_execution": {}}"#)]
fn the_post_merge_pass_leaves_tools_that_do_not_conflict(#[case] source: &str) {
    let mut tools: Vec<Tool> = parse(&format!("[{source}]"));
    assert!(!drop_mixed_search_tools(&mut tools, false));
    assert_eq!(tools.len(), 2);
}

#[test]
fn the_post_merge_pass_drops_search_tools_and_reports_it() {
    let mut tools: Vec<Tool> = parse(
        r#"[
            {"function_declarations": [{"name": "f"}]},
            {"googleSearch": {}},
            {"googleSearchRetrieval": {}},
            {"enterpriseWebSearch": {}},
            {"url_context": {}},
            {"code_execution": {}}
        ]"#,
    );
    assert!(drop_mixed_search_tools(&mut tools, false));
    assert_eq!(
        as_json(&tools),
        parse::<Json>(r#"[{"function_declarations": [{"name": "f"}]}, {"code_execution": {}}]"#)
    );
}

#[test]
fn the_post_merge_pass_keeps_search_tools_when_server_side_invocations_are_enabled() {
    let mut tools: Vec<Tool> =
        parse(r#"[{"function_declarations": [{"name": "f"}]}, {"googleSearch": {}}]"#);
    assert!(!drop_mixed_search_tools(&mut tools, true));
    assert_eq!(tools.len(), 2);
}

#[test]
fn the_post_merge_pass_ignores_an_empty_tool_list() {
    let mut tools: Vec<Tool> = Vec::new();
    assert!(!drop_mixed_search_tools(&mut tools, false));
}

#[test]
fn maps_coordinates_move_to_the_retrieval_config_as_a_pair() {
    let input: Vec<Object> = parse(
        r#"[{"googleMaps": {"enableWidget": true, "latitude": 12.5, "longitude": -7.25, "languageCode": "en-GB"}}]"#,
    );
    let existing = ToolConfig::from(FunctionCallingConfig {
        mode: CallingMode::Any,
        allowed_function_names: None,
    });
    let mapped = map_tools(&input, false, Some(existing), &Settings::default()).unwrap();
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(r#"[{"googleMaps": {"enableWidget": true}}]"#)
    );
    assert_eq!(
        as_json(&mapped.tool_config),
        parse::<Json>(
            r#"{"functionCallingConfig": {"mode": "ANY"}, "retrievalConfig": {"latLng": {"latitude": 12.5, "longitude": -7.25}, "languageCode": "en-GB"}}"#
        )
    );
}

#[test]
fn maps_treat_zero_coordinates_as_supplied() {
    let input: Vec<Object> = parse(r#"[{"googleMaps": {"latitude": 0, "longitude": 0}}]"#);
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tool_config),
        parse::<Json>(r#"{"retrievalConfig": {"latLng": {"latitude": 0, "longitude": 0}}}"#)
    );
}

#[test]
fn maps_without_a_coordinate_pair_lose_the_flat_language_code() {
    let input: Vec<Object> =
        parse(r#"[{"googleMaps": {"enableWidget": false, "languageCode": "fr"}}]"#);
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(r#"[{"googleMaps": {"enableWidget": false}}]"#)
    );
    assert_eq!(mapped.tool_config, None);
}

#[test]
fn maps_keep_a_nested_retrieval_config_in_the_tool() {
    let input: Vec<Object> = parse(
        r#"[{"googleMaps": {"enableWidget": true, "retrievalConfig": {"latLng": {"latitude": 1, "longitude": 2}}}}]"#,
    );
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(
            r#"[{"googleMaps": {"enableWidget": true, "retrievalConfig": {"latLng": {"latitude": 1, "longitude": 2}}}}]"#
        )
    );
    assert_eq!(mapped.tool_config, None);
}

#[test]
fn maps_treat_null_coordinates_as_absent() {
    let input: Vec<Object> =
        parse(r#"[{"googleMaps": {"latitude": null, "longitude": 2, "languageCode": "en"}}]"#);
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(r#"[{"googleMaps": {}}]"#)
    );
    assert_eq!(mapped.tool_config, None);
}

#[rstest]
#[case::browser(
    r#"{"environment": "browser"}"#,
    r#"{"environment": "ENVIRONMENT_BROWSER"}"#
)]
#[case::unspecified(
    r#"{"environment": "unspecified"}"#,
    r#"{"environment": "ENVIRONMENT_UNSPECIFIED"}"#
)]
#[case::canonical_browser(
    r#"{"environment": "ENVIRONMENT_BROWSER"}"#,
    r#"{"environment": "ENVIRONMENT_BROWSER"}"#
)]
#[case::canonical_unspecified(
    r#"{"environment": "ENVIRONMENT_UNSPECIFIED"}"#,
    r#"{"environment": "ENVIRONMENT_UNSPECIFIED"}"#
)]
#[case::invalid_environment(r#"{"environment": "bogus", "enableWidget": true}"#, "{}")]
#[case::missing_environment(
    r#"{"excluded_predefined_functions": ["a"]}"#,
    r#"{"excludedPredefinedFunctions": ["a"]}"#
)]
#[case::both_fields_in_camel_case(
    r#"{"environment": "browser", "excludedPredefinedFunctions": ["b"]}"#,
    r#"{"environment": "ENVIRONMENT_BROWSER", "excludedPredefinedFunctions": ["b"]}"#
)]
#[case::null_environment(r#"{"environment": null}"#, "{}")]
fn computer_use_configuration_is_transformed(#[case] config: &str, #[case] expected: &str) {
    let transformed = transform_computer_use(&parse::<Object>(config));
    assert_eq!(as_json(&transformed), parse::<Json>(expected));
}

#[test]
fn a_computer_use_tool_with_a_null_config_gets_an_empty_one() {
    let mapped = mapped(&tools(r#"[{"computerUse": null}]"#));
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(r#"[{"computerUse": {}}]"#)
    );
}

#[rstest]
#[case::none("none", r#"{"functionCallingConfig": {"mode": "NONE"}}"#)]
#[case::required("required", r#"{"functionCallingConfig": {"mode": "ANY"}}"#)]
#[case::auto("auto", r#"{"functionCallingConfig": {"mode": "AUTO"}}"#)]
#[case::function(
    r#"{"type": "function", "function": {"name": "f"}}"#,
    r#"{"functionCallingConfig": {"mode": "ANY", "allowed_function_names": ["f"]}}"#
)]
#[case::dict_without_function(
    "{}",
    r#"{"functionCallingConfig": {"mode": "ANY", "allowed_function_names": [""]}}"#
)]
#[case::dict_without_name(
    r#"{"function": {}}"#,
    r#"{"functionCallingConfig": {"mode": "ANY", "allowed_function_names": [""]}}"#
)]
#[case::null_function(
    r#"{"function": null}"#,
    r#"{"functionCallingConfig": {"mode": "ANY", "allowed_function_names": [""]}}"#
)]
fn tool_choice_maps_the_supported_forms(#[case] choice: &str, #[case] expected: &str) {
    let mapped = map_tool_choice(&parse::<Json>(choice)).unwrap();
    assert_eq!(as_json(&mapped), parse::<Json>(expected));
}

#[rstest]
#[case::unknown_string(r#""mandatory""#)]
#[case::number("5")]
#[case::list(r#"["auto"]"#)]
#[case::boolean("true")]
#[case::null("null")]
fn tool_choice_rejects_unsupported_values(#[case] choice: &str) {
    assert!(matches!(
        map_tool_choice(&parse::<Json>(choice)),
        Err(GeminiToolsError::UnsupportedToolChoice)
    ));
}

#[test]
fn tool_choice_rejects_a_function_that_is_not_an_object() {
    assert!(matches!(
        map_tool_choice(&parse::<Json>(r#"{"function": "f"}"#)),
        Err(GeminiToolsError::ToolChoiceField { field: "function" })
    ));
}

#[test]
fn tool_choice_rejects_a_function_name_that_is_not_a_string() {
    assert!(matches!(
        map_tool_choice(&parse::<Json>(r#"{"function": {"name": 5}}"#)),
        Err(GeminiToolsError::ToolChoiceField { field: "name" })
    ));
}

#[rstest]
#[case::auto("auto", "priority")]
#[case::upper_auto("AUTO", "priority")]
#[case::mixed_auto("Auto", "priority")]
#[case::standard("STANDARD", "standard")]
#[case::empty("", "")]
fn service_tier_maps_auto_to_priority_and_lowercases_the_rest(
    #[case] value: &str,
    #[case] expected: &str,
) {
    assert_eq!(map_service_tier(value), expected);
}

#[test]
fn cleaning_removes_strict_and_false_additional_properties_recursively() {
    let mut tool = parse::<Object>(
        r#"{
            "strict": true,
            "additionalProperties": false,
            "nested": {
                "strict": false,
                "additionalProperties": true,
                "list": [{"additionalProperties": false, "strict": true}]
            }
        }"#,
    );
    remove_unsupported_fields(&mut tool);
    assert_eq!(
        as_json(&tool),
        parse::<Json>(r#"{"nested": {"additionalProperties": true, "list": [{}]}}"#)
    );
}

#[test]
fn function_parameters_are_shaped_for_vertex() {
    let input: Vec<Object> = parse(
        r#"[{"type": "function", "function": {
            "name": "f",
            "strict": true,
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"a": {"type": "string", "strict": true}}
            }
        }}]"#,
    );
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(
            r#"[{"function_declarations": [{"name": "f", "parameters": {"type": "object", "properties": {"a": {"type": "string"}}}}]}]"#
        )
    );
}

#[test]
fn empty_parameters_become_an_object_schema() {
    let input: Vec<Object> =
        parse(r#"[{"type": "function", "function": {"name": "f", "parameters": ""}}]"#);
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(
            r#"[{"function_declarations": [{"name": "f", "parameters": {"type": "object"}}]}]"#
        )
    );
}

#[test]
fn null_parameters_are_omitted() {
    let input: Vec<Object> =
        parse(r#"[{"type": "function", "function": {"name": "f", "parameters": null}}]"#);
    let mapped = mapped(&input);
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(r#"[{"function_declarations": [{"name": "f"}]}]"#)
    );
}

#[test]
fn unknown_tools_are_dropped_and_counted() {
    let input: Vec<Object> = parse(r#"[{"whatever": {}}, {"type": "mystery", "x": 1}]"#);
    let mapped = mapped(&input);
    assert!(mapped.tools.is_empty());
    assert_eq!(mapped.warnings, 2);
}

#[test]
fn a_null_search_value_emits_no_tool() {
    let mapped = mapped(&tools(r#"[{"googleSearch": null}]"#));
    assert!(mapped.tools.is_empty());
    assert_eq!(mapped.warnings, 0);
}

#[test]
fn a_null_maps_value_emits_no_tool() {
    let mapped = mapped(&tools(r#"[{"googleMaps": null}]"#));
    assert!(mapped.tools.is_empty());
}

fn declaration_error(source: &str) -> GeminiToolsError {
    map_tools(&tools(source), false, None, &Settings::default()).unwrap_err()
}

#[test]
fn a_missing_function_name_is_rejected() {
    assert!(matches!(
        declaration_error(r#"[{"type": "function", "function": {"description": "d"}}]"#),
        GeminiToolsError::FunctionField { field: "name" }
    ));
    assert!(matches!(
        declaration_error(r#"[{"type": "function", "function": {"name": null}}]"#),
        GeminiToolsError::FunctionField { field: "name" }
    ));
    assert!(matches!(
        declaration_error(r#"[{"type": "function", "function": {"name": 5}}]"#),
        GeminiToolsError::FunctionField { field: "name" }
    ));
}

#[test]
fn a_description_that_is_not_a_string_is_rejected() {
    assert!(matches!(
        declaration_error(r#"[{"type": "function", "function": {"name": "f", "description": 5}}]"#),
        GeminiToolsError::FunctionField {
            field: "description",
        }
    ));
}

#[test]
fn parameters_that_are_not_a_schema_are_rejected() {
    assert!(matches!(
        declaration_error(
            r#"[{"type": "function", "function": {"name": "f", "parameters": [1]}}]"#
        ),
        GeminiToolsError::FunctionField {
            field: "parameters",
        }
    ));
    assert!(matches!(
        declaration_error(
            r#"[{"type": "function", "function": {"name": "f", "parameters": "abc"}}]"#
        ),
        GeminiToolsError::FunctionField {
            field: "parameters",
        }
    ));
}

#[test]
fn a_function_that_is_not_an_object_is_rejected() {
    let error = map_tools(
        &tools(r#"[{"function": "f"}]"#),
        false,
        None,
        &Settings::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        GeminiToolsError::FunctionField { field: "function" }
    ));
}

#[rstest]
#[case::search(r#"[{"googleSearch": "x"}]"#)]
#[case::maps(r#"[{"googleMaps": "x"}]"#)]
#[case::computer(r#"[{"computerUse": "x"}]"#)]
#[case::retrieval(r#"[{"googleSearchRetrieval": []}]"#)]
fn a_special_tool_value_that_is_not_an_object_is_rejected(#[case] source: &str) {
    assert!(matches!(
        map_tools(&tools(source), false, None, &Settings::default()),
        Err(GeminiToolsError::ToolValue { .. })
    ));
}

#[test]
fn a_schema_past_the_configured_depth_is_an_error() {
    let settings = Settings {
        max_recurse_depth: 0,
    };
    let input: Vec<Object> = parse(
        r#"[{"type": "function", "function": {"name": "f", "parameters": {"properties": {"a": {"type": "string"}}}}}]"#,
    );
    assert!(matches!(
        map_tools(&input, false, None, &settings),
        Err(GeminiToolsError::Shape { .. })
    ));
}

#[rstest]
#[case::camel("googleSearch", "google_search")]
#[case::several_words("enterpriseWebSearch", "enterprise_web_search")]
#[case::already_snake("url_context", "url_context")]
#[case::leading_underscore("_Foo", "foo")]
#[case::empty("", "")]
fn camel_case_names_become_underscore_names(#[case] name: &str, #[case] expected: &str) {
    assert_eq!(underscore_name(name), expected);
}

#[test]
fn tool_values_prefer_the_camel_case_key_and_treat_null_as_absent() {
    let snake_only = parse::<Object>(r#"{"googleSearch": null, "google_search": {"a": 1}}"#);
    assert_eq!(
        as_json(&tool_value(&snake_only, "googleSearch")),
        parse::<Json>(r#"{"a": 1}"#)
    );
    let both = parse::<Object>(r#"{"googleSearch": {"b": 2}, "google_search": {"a": 1}}"#);
    assert_eq!(
        as_json(&tool_value(&both, "googleSearch")),
        parse::<Json>(r#"{"b": 2}"#)
    );
    assert_eq!(tool_value(&both, "computerUse"), None);
}

#[test]
fn openai_web_search_tools_become_google_search() {
    let preview = transform_type(&parse::<Object>(
        r#"{"type": "web_search_preview", "search_context_size": "low"}"#,
    ));
    assert_eq!(as_json(&preview), parse::<Json>(r#"{"googleSearch": {}}"#));
    let search = transform_type(&parse::<Object>(r#"{"type": "web_search"}"#));
    assert_eq!(as_json(&search), parse::<Json>(r#"{"googleSearch": {}}"#));
}

#[test]
fn a_generic_type_field_is_stripped() {
    let stripped = transform_type(&parse::<Object>(r#"{"type": "function", "name": "f"}"#));
    assert_eq!(as_json(&stripped), parse::<Json>(r#"{"name": "f"}"#));
}

#[test]
fn a_tool_without_a_type_is_left_alone() {
    let untouched = transform_type(&parse::<Object>(r#"{"googleSearch": {"a": 1}}"#));
    assert_eq!(
        as_json(&untouched),
        parse::<Json>(r#"{"googleSearch": {"a": 1}}"#)
    );
}

#[test]
fn an_openai_web_search_type_tool_becomes_a_search_tool_in_the_list() {
    let mapped = mapped(&tools(
        r#"[{"type": "web_search", "search_context_size": "low"}]"#,
    ));
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(r#"[{"googleSearch": {}}]"#)
    );
}

#[test]
fn a_computer_use_type_tool_becomes_a_computer_use_tool() {
    let mapped = mapped(&tools(
        r#"[{"type": "computer_use", "environment": "browser", "excluded_predefined_functions": ["x"]}]"#,
    ));
    assert_eq!(
        as_json(&mapped.tools),
        parse::<Json>(
            r#"[{"computerUse": {"environment": "ENVIRONMENT_BROWSER", "excludedPredefinedFunctions": ["x"]}}]"#
        )
    );
}

#[test]
fn extract_maps_returns_the_cleaned_config_and_the_retrieval_pair() {
    let (cleaned, retrieval) = extract_maps(&parse::<Object>(
        r#"{"enableWidget": true, "latitude": 1, "longitude": 2}"#,
    ));
    assert_eq!(
        as_json(&cleaned),
        parse::<Json>(r#"{"enableWidget": true}"#)
    );
    assert_eq!(
        as_json(&retrieval),
        parse::<Json>(r#"{"latLng": {"latitude": 1, "longitude": 2}}"#)
    );
}

#[test]
fn a_tool_choice_function_object_keeps_only_its_name() {
    let mapped = map_tool_choice(&parse::<Json>(
        r#"{"function": {"name": "f", "arguments": "{}"}}"#,
    ))
    .unwrap();
    assert_eq!(
        as_json(&mapped),
        parse::<Json>(
            r#"{"functionCallingConfig": {"mode": "ANY", "allowed_function_names": ["f"]}}"#
        )
    );
}

#[test]
fn function_declarations_carry_a_description_and_no_parameters_when_absent() {
    let declaration = FunctionDeclaration {
        name: "f".to_owned(),
        description: Some("d".to_owned()),
        parameters: None,
    };
    assert_eq!(
        as_json(&declaration),
        parse::<Json>(r#"{"name": "f", "description": "d"}"#)
    );
}
