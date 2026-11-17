use std::slice::from_ref;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_anthropic::anthropic_capability_catalog::forced_tool::ForcedToolChoiceError;
use betterlite_models::decisions::Json;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use indexmap::IndexMap;

use super::support::{CATALOG, Fixture, LLAMA, MODEL, NOVA, json, object};
use crate::converse_tools::additional_properties::add_additional_properties;
use crate::converse_tools::computer_use::{
    is_computer_use_tool_used, separate_computer_use_tools, transform_computer_use_tools,
};
use crate::converse_tools::error::ConverseToolsError;
use crate::converse_tools::json_mode::{
    append_tools, apply_tool_call_transformation, create_json_tool_call,
};
use crate::converse_tools::media_types::{
    SUPPORTED_DOCUMENT_TYPES, SUPPORTED_IMAGE_TYPES, SUPPORTED_VIDEO_TYPES,
};
use crate::converse_tools::tool_choice::{ToolChoiceValuesBlock, map_tool_choice_values};
use crate::converse_tools::tool_spec::BedrockToolSpec;

fn choice(
    model: &str,
    tool_choice: &str,
    drop: DropParams,
    catalogs: &CatalogSources<'_>,
) -> Result<Option<ToolChoiceValuesBlock>, ConverseToolsError> {
    let mut names = ToolNameMap::new();
    map_tool_choice_values(model, &json(tool_choice), drop, catalogs, &mut names)
}

pub(crate) fn tool_choice_values() {
    let cost = json(CATALOG);
    let fixture = Fixture::new(&cost);
    let catalogs = fixture.catalogs();
    string_choices(&catalogs);
    none_choices(&catalogs);
    unknown_choices(&catalogs);
    dict_choices(&catalogs);
    downgraded_dict_choices(&catalogs);
    malformed_choices(&catalogs);
}

fn string_choices(catalogs: &CatalogSources<'_>) {
    let no_drop = DropParams::default();
    let drop_call = DropParams {
        global: false,
        call: true,
    };
    let drop_global = DropParams {
        global: true,
        call: false,
    };
    assert_eq!(
        choice(MODEL, r#""auto""#, no_drop, catalogs).expect("auto maps"),
        Some(object(r#"{"auto": {}}"#))
    );
    assert_eq!(
        choice(MODEL, r#""required""#, no_drop, catalogs).expect("required maps"),
        Some(object(r#"{"any": {}}"#))
    );
    assert_eq!(
        choice(NOVA, r#""required""#, drop_call, catalogs).expect("a dropped required maps"),
        Some(object(r#"{"auto": {}}"#))
    );
    assert_eq!(
        choice(NOVA, r#""required""#, drop_global, catalogs).expect("a globally dropped required"),
        Some(object(r#"{"auto": {}}"#))
    );
    let downgraded = choice(NOVA, r#""required""#, no_drop, catalogs).expect_err("no drop asked");
    assert!(matches!(
        downgraded,
        ConverseToolsError::ForcedTool(ForcedToolChoiceError::Unsupported { .. })
    ));
    assert_eq!(downgraded.status_code(), Some(400));
}

fn none_choices(catalogs: &CatalogSources<'_>) {
    let no_drop = DropParams::default();
    let drop_call = DropParams {
        global: false,
        call: true,
    };
    let drop_global = DropParams {
        global: true,
        call: false,
    };
    assert_eq!(
        choice(MODEL, r#""none""#, drop_global, catalogs).expect("a global drop removes none"),
        None
    );
    assert_eq!(
        choice(MODEL, r#""none""#, drop_call, catalogs).expect("a call drop removes none"),
        None
    );
    let kept_none = choice(MODEL, r#""none""#, no_drop, catalogs).expect_err("none is unsupported");
    assert!(matches!(kept_none, ConverseToolsError::UnsupportedNone));
    assert_eq!(kept_none.status_code(), Some(400));
}

fn unknown_choices(catalogs: &CatalogSources<'_>) {
    let drop_global = DropParams {
        global: true,
        call: false,
    };
    let unknown = choice(MODEL, r#""bogus""#, drop_global, catalogs).expect_err("not dropped");
    assert_eq!(unknown.status_code(), Some(400));
    let ConverseToolsError::UnsupportedChoice { value } = unknown else {
        panic!("a bogus string is an unsupported choice");
    };
    assert_eq!(value, "bogus");
    assert_eq!(
        choice(MODEL, "7", drop_global, catalogs)
            .expect_err("a number is not a tool choice")
            .status_code(),
        Some(400)
    );
    assert_eq!(
        choice(MODEL, "null", DropParams::default(), catalogs)
            .expect_err("null is not a tool choice")
            .status_code(),
        Some(400)
    );
}

fn dict_choices(catalogs: &CatalogSources<'_>) {
    let cases = [
        (
            r#"{"type": "function", "function": {"name": "get weather!"}}"#,
            r#"{"tool": {"name": "get_weather_"}}"#,
        ),
        (r#"{"type": "auto"}"#, r#"{"tool": {"name": ""}}"#),
        (
            r#"{"function": {"name": null}}"#,
            r#"{"tool": {"name": null}}"#,
        ),
    ];
    for (tool_choice, expected) in cases {
        assert_eq!(
            choice(MODEL, tool_choice, DropParams::default(), catalogs)
                .expect("the dict maps to a tool choice"),
            Some(object(expected))
        );
    }
}

fn downgraded_dict_choices(catalogs: &CatalogSources<'_>) {
    let no_drop = DropParams::default();
    let drop_call = DropParams {
        global: false,
        call: true,
    };
    assert_eq!(
        choice(
            NOVA,
            r#"{"function": {"name": "specific"}}"#,
            drop_call,
            catalogs
        )
        .expect("the downgraded dict maps"),
        Some(object(r#"{"auto": {}}"#))
    );
    assert_eq!(
        choice(
            NOVA,
            r#"{"function": {"name": "specific"}}"#,
            no_drop,
            catalogs
        )
        .expect_err("forced tool use is refused")
        .status_code(),
        Some(400)
    );
}

fn malformed_choices(catalogs: &CatalogSources<'_>) {
    let no_drop = DropParams::default();
    let bad_function = choice(MODEL, r#"{"function": 5}"#, no_drop, catalogs)
        .expect_err("a non-object function is refused");
    assert!(matches!(
        bad_function,
        ConverseToolsError::ToolChoiceFunction
    ));
    let bad_name = choice(MODEL, r#"{"function": {"name": 5}}"#, no_drop, catalogs)
        .expect_err("a non-string name is refused");
    assert!(matches!(bad_name, ConverseToolsError::ToolChoiceName));
    assert_eq!(
        ConverseToolsError::ToolChoiceName.status_code(),
        Some(400),
        "a malformed name is a bad request"
    );
}

pub(crate) fn computer_use_tool_split() {
    let tools = json(
        r#"[
            {"type": "computer_use_preview", "name": "computer"},
            {"type": "computer_20250124", "function": {"name": "comp", "parameters": {"type": "object", "name": "overridden"}}},
            {"type": "bash_20250124"},
            {"type": "text_editor_20250124"},
            {"type": "bash_20250124", "name": ""},
            {"type": "computer_20250124"},
            {"type": "computer_20250124", "name": null},
            {"type": "custom_tool", "description": "kept"},
            {"name": "nameless"}
        ]"#,
    );
    let Json::List(entries) = tools else {
        panic!("the fixture is a list");
    };
    detector_answers(&entries);
    partition_answers(&entries);
    transform_answers(&entries);
    tool_errors();
}

fn detector_answers(tools: &[Json]) {
    assert!(!is_computer_use_tool_used(None).expect("no tools is false"));
    assert!(is_computer_use_tool_used(Some(tools)).expect("a computer tool is found"));
    assert!(
        !is_computer_use_tool_used(Some(&[json(r#"{"name": "plain"}"#)]))
            .expect("a tool without a type is not a computer tool")
    );
    assert!(
        !is_computer_use_tool_used(Some(&[json(r#"{"type": "custom_tool"}"#)]))
            .expect("a custom tool is not a computer tool")
    );
    assert!(
        is_computer_use_tool_used(Some(&[
            json(r#"{"type": "custom_tool"}"#),
            json(r#"{"type": "bash_20250124"}"#)
        ]))
        .expect("a later computer tool is found")
    );
}

fn partition_answers(tools: &[Json]) {
    let (computer, regular) =
        separate_computer_use_tools(tools).expect("the tools separate cleanly");
    assert_eq!(computer.len(), 7);
    assert_eq!(regular.len(), 2);
    assert_eq!(computer.first(), tools.first());
    assert_eq!(
        regular,
        vec![
            json(r#"{"type": "custom_tool", "description": "kept"}"#),
            json(r#"{"name": "nameless"}"#)
        ]
    );
}

fn transform_answers(tools: &[Json]) {
    let transformed = transform_computer_use_tools(tools).expect("the tools transform cleanly");
    assert_eq!(
        transformed,
        vec![
            json(r#"{"type": "computer_use_preview", "name": "computer"}"#),
            json(r#"{"type": "object", "name": "overridden"}"#),
            json(r#"{"type": "bash_20250124", "name": "bash"}"#),
            json(r#"{"type": "text_editor_20250124", "name": "str_replace_editor"}"#),
            json(r#"{"type": "bash_20250124", "name": ""}"#),
            json(r#"{"type": "computer_20250124"}"#),
            json(r#"{"type": "computer_20250124", "name": null}"#),
            json(r#"{"type": "custom_tool", "description": "kept"}"#),
            json(r#"{"name": "nameless"}"#),
        ]
    );
    assert_eq!(
        transform_computer_use_tools(&[json(r#"{"type": "computer_x", "function": {}}"#)])
            .expect("the tool transforms cleanly"),
        vec![json(r#"{"type": "computer_x", "name": "computer"}"#)]
    );
}

fn tool_errors() {
    let not_object =
        is_computer_use_tool_used(Some(&[json("5")])).expect_err("a tool must be an object");
    assert!(matches!(not_object, ConverseToolsError::ToolNotAnObject));
    let bad_type = is_computer_use_tool_used(Some(&[json(r#"{"type": 5}"#)]))
        .expect_err("a tool type must be a string");
    assert!(matches!(bad_type, ConverseToolsError::ToolType));
    let bad_function =
        transform_computer_use_tools(&[json(r#"{"type": "computer_x", "function": 5}"#)])
            .expect_err("a function must be an object");
    assert!(matches!(bad_function, ConverseToolsError::ToolFunction));
    let bad_parameters = transform_computer_use_tools(&[json(
        r#"{"type": "computer_x", "function": {"parameters": null}}"#,
    )])
    .expect_err("parameters must be an object");
    assert!(matches!(bad_parameters, ConverseToolsError::ToolParameters));
    let non_string_type = transform_computer_use_tools(&[json(r#"{"type": 5}"#)])
        .expect_err("a tool type must be a string");
    assert!(matches!(non_string_type, ConverseToolsError::ToolType));
}

pub(crate) fn json_mode_tool() {
    json_call_shapes();
    tool_appending();
    let tool = create_json_tool_call(Some(&json("{}")), None);
    fake_stream_flags(&tool);
}

fn json_call_shapes() {
    assert_eq!(
        create_json_tool_call(None, None),
        json(
            r#"{"type": "function", "function": {"name": "json_tool_call", "parameters": {"type": "object", "additionalProperties": true, "properties": {}}}}"#
        )
    );
    assert_eq!(
        create_json_tool_call(Some(&json("5")), None),
        json(r#"{"type": "function", "function": {"name": "json_tool_call", "parameters": 5}}"#)
    );
    assert_eq!(
        create_json_tool_call(Some(&json("{}")), Some("")),
        json(
            r#"{"type": "function", "function": {"name": "json_tool_call", "parameters": {}, "description": ""}}"#
        )
    );
    assert_eq!(
        create_json_tool_call(Some(&json("{}")), Some("why")),
        json(
            r#"{"type": "function", "function": {"name": "json_tool_call", "parameters": {}, "description": "why"}}"#
        )
    );
}

fn tool_appending() {
    let tool = create_json_tool_call(Some(&json("{}")), None);
    let mut params = object(r#"{"tools": [{"first": 1}, {"first": 1}]}"#);
    append_tools(&mut params, vec![tool.clone()]).expect("a list extends");
    assert_eq!(
        params,
        object(
            r#"{"tools": [{"first": 1}, {"first": 1}, {"type": "function", "function": {"name": "json_tool_call", "parameters": {}}}]}"#
        )
    );
    let mut absent = object("{}");
    append_tools(&mut absent, vec![tool.clone()]).expect("an absent key is set");
    assert_eq!(absent.get("tools"), Some(&Json::List(vec![tool.clone()])));
    let not_list = append_tools(&mut object(r#"{"tools": 5}"#), vec![tool])
        .expect_err("a non-list tools value is refused");
    assert!(matches!(not_list, ConverseToolsError::ToolsNotList));
}

fn fake_stream_flags(tool: &Json) {
    let mut streamed = object("{}");
    apply_tool_call_transformation(from_ref(tool), LLAMA, &json("true"), &mut streamed)
        .expect("the tools apply");
    assert_eq!(streamed.get("fake_stream"), Some(&Json::Bool(true)));
    let mut prefixed = object("{}");
    apply_tool_call_transformation(
        from_ref(tool),
        "us.meta.llama3-3-70b-instruct-v1:0",
        &json("true"),
        &mut prefixed,
    )
    .expect("the tools apply");
    assert_eq!(prefixed.get("fake_stream"), Some(&Json::Bool(true)));
    for stream in ["false", "null", "1", r#""true""#] {
        let mut params = object("{}");
        apply_tool_call_transformation(from_ref(tool), LLAMA, &json(stream), &mut params)
            .expect("the tools apply");
        assert_eq!(
            params.get("fake_stream"),
            None,
            "stream {stream} is not true"
        );
    }
    let mut other = object("{}");
    apply_tool_call_transformation(
        from_ref(tool),
        "amazon.nova-pro-v1:0",
        &json("true"),
        &mut other,
    )
    .expect("the tools apply");
    assert_eq!(other.get("fake_stream"), None);
}

pub(crate) fn additional_properties_recursion() {
    primitive_values();
    walked_positions();
    definitions_and_unions();
    deep_schema();
    source_is_not_mutated();
}

fn primitive_values() {
    assert_eq!(
        add_additional_properties(&json(r#"{"type": "object"}"#)),
        json(r#"{"type": "object", "additionalProperties": false}"#)
    );
    assert_eq!(
        add_additional_properties(&json(r#"{"type": "object", "additionalProperties": true}"#)),
        json(r#"{"type": "object", "additionalProperties": true}"#)
    );
    assert_eq!(
        add_additional_properties(&json(r#"{"type": "object", "additionalProperties": null}"#)),
        json(r#"{"type": "object", "additionalProperties": null}"#)
    );
    assert_eq!(
        add_additional_properties(&json(r#"{"type": "string"}"#)),
        json(r#"{"type": "string"}"#)
    );
    assert_eq!(add_additional_properties(&json("5")), json("5"));
    assert_eq!(add_additional_properties(&json("[1]")), json("[1]"));
}

fn walked_positions() {
    assert_eq!(
        add_additional_properties(&json(
            r#"{"type": "object", "properties": {"a": {"type": "object"}, "b": 5}}"#
        )),
        json(
            r#"{"type": "object", "properties": {"a": {"type": "object", "additionalProperties": false}, "b": 5}, "additionalProperties": false}"#
        )
    );
    assert_eq!(
        add_additional_properties(&json(r#"{"type": "array", "items": {"type": "object"}}"#)),
        json(r#"{"type": "array", "items": {"type": "object", "additionalProperties": false}}"#)
    );
    assert_eq!(
        add_additional_properties(&json(
            r#"{"items": [{"type": "object"}], "not": {"type": "object"}, "patternProperties": {"x": {"type": "object"}}, "additionalProperties": {"type": "object"}}"#
        )),
        json(
            r#"{"items": [{"type": "object"}], "not": {"type": "object"}, "patternProperties": {"x": {"type": "object"}}, "additionalProperties": {"type": "object"}}"#
        )
    );
}

fn definitions_and_unions() {
    assert_eq!(
        add_additional_properties(&json(
            r#"{"$defs": {"a": {"type": "object"}, "b": 1}, "definitions": {"c": {"type": "object"}}}"#
        )),
        json(
            r#"{"$defs": {"a": {"type": "object", "additionalProperties": false}, "b": 1}, "definitions": {"c": {"type": "object", "additionalProperties": false}}}"#
        )
    );
    assert_eq!(
        add_additional_properties(&json(
            r#"{"anyOf": [{"type": "object"}, 5], "allOf": [{"type": "object"}], "oneOf": [{"type": "object"}]}"#
        )),
        json(
            r#"{"anyOf": [{"type": "object", "additionalProperties": false}, 5], "allOf": [{"type": "object", "additionalProperties": false}], "oneOf": [{"type": "object", "additionalProperties": false}]}"#
        )
    );
}

// Each iteration adds the node and its properties map, so 63 reaches serde_json's 127-container parse limit.
const DEEP_LEVELS: usize = 63;

fn deep_schema() {
    let mut deep = Json::Object(object(r#"{"type": "object"}"#));
    for _ in 0_usize..DEEP_LEVELS {
        let mut node = object(r#"{"type": "object"}"#);
        node.insert(
            "properties".to_owned(),
            Json::Object(IndexMap::from([("a".to_owned(), deep)])),
        );
        deep = Json::Object(node);
    }
    let patched_deep = add_additional_properties(&deep);
    let mut level = &patched_deep;
    let mut depth = 0_usize;
    while let Json::Object(fields) = level {
        assert_eq!(
            fields.get("additionalProperties"),
            Some(&Json::Bool(false)),
            "level {depth} of a deep schema is patched"
        );
        let next = fields
            .get("properties")
            .and_then(|properties| match properties {
                Json::Object(map) => map.get("a"),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    None
                }
            });
        let Some(child) = next else {
            break;
        };
        level = child;
        depth = depth.saturating_add(1);
    }
    assert_eq!(
        depth, DEEP_LEVELS,
        "every level of the deep schema is patched"
    );
}

fn source_is_not_mutated() {
    let source = json(r#"{"type": "object", "properties": {"a": {"type": "object"}}}"#);
    let patched = add_additional_properties(&source);
    assert_eq!(
        source,
        json(r#"{"type": "object", "properties": {"a": {"type": "object"}}}"#),
        "the source schema is not mutated"
    );
    assert_ne!(patched, source);
}

pub(crate) fn strict_tool_spec() {
    plain_blocks();
    null_and_extra_blocks();
    strict_flags();
    additional_properties_filtering();
    parameter_errors();
}

fn plain_blocks() {
    let parameters = json(r#"{"type": "object", "properties": {"a": 1}, "required": ["a"]}"#);
    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "desc",
            parameters: &parameters,
            strict: None,
            supports_strict_tools: false,
        }
        .into_block()
        .expect("the plain spec builds"),
        json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {"a": 1}, "required": ["a"]}}, "name": "tool", "description": "desc"}}"#
        )
    );
    let minimal = json(r#"{"type": "object"}"#);
    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "",
            parameters: &minimal,
            strict: None,
            supports_strict_tools: false,
        }
        .into_block()
        .expect("the minimal spec builds"),
        json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "tool", "description": ""}}"#
        )
    );
}

fn null_and_extra_blocks() {
    let nulls = json(r#"{"type": null, "properties": null, "required": null}"#);
    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "d",
            parameters: &nulls,
            strict: None,
            supports_strict_tools: false,
        }
        .into_block()
        .expect("the null spec builds"),
        json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": null, "properties": null, "required": null}}, "name": "tool", "description": "d"}}"#
        )
    );
    let extras = json(r#"{"type": "object", "$defs": {"x": 1}, "description": "dropped"}"#);
    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "d",
            parameters: &extras,
            strict: None,
            supports_strict_tools: false,
        }
        .into_block()
        .expect("the extra keys are dropped"),
        json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "tool", "description": "d"}}"#
        )
    );
}

fn strict_flags() {
    let parameters = json(r#"{"type": "object", "additionalProperties": false, "properties": {}}"#);
    for strict in [Some(true), Some(false), None] {
        let block = BedrockToolSpec {
            name: "tool",
            description: "d",
            parameters: &parameters,
            strict,
            supports_strict_tools: true,
        }
        .into_block()
        .expect("the strict spec builds");
        assert_strict_block(&block, strict, Some(false));
    }
    let unsupported = BedrockToolSpec {
        name: "tool",
        description: "d",
        parameters: &parameters,
        strict: Some(true),
        supports_strict_tools: false,
    }
    .into_block()
    .expect("the plain spec builds");
    assert_strict_block(&unsupported, None, None);
}

fn assert_strict_block(block: &Json, strict: Option<bool>, additional: Option<bool>) {
    let Json::Object(fields) = block else {
        panic!("the block is an object");
    };
    let Json::Object(spec) = fields.get("toolSpec").expect("a tool spec") else {
        panic!("the spec is an object");
    };
    let expected_strict = (strict == Some(true)).then_some(Json::Bool(true));
    assert_eq!(spec.get("strict"), expected_strict.as_ref(), "{strict:?}");
    let Json::Object(schema) = spec.get("inputSchema").expect("an input schema") else {
        panic!("the input schema is an object");
    };
    let Json::Object(json_schema) = schema.get("json").expect("a json schema") else {
        panic!("the json schema is an object");
    };
    assert_eq!(
        json_schema.get("additionalProperties"),
        additional.map(Json::Bool).as_ref(),
        "{additional:?}"
    );
}

fn additional_properties_filtering() {
    let null_additional = json(r#"{"type": "object", "additionalProperties": null}"#);
    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "d",
            parameters: &null_additional,
            strict: Some(true),
            supports_strict_tools: true,
        }
        .into_block()
        .expect("the filtered additionalProperties is dropped"),
        json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "tool", "description": "d", "strict": true}}"#
        )
    );
}

fn parameter_errors() {
    let non_object = json("5");
    let parameters_error = BedrockToolSpec {
        name: "tool",
        description: "d",
        parameters: &non_object,
        strict: None,
        supports_strict_tools: false,
    }
    .into_block()
    .expect_err("parameters must be an object");
    assert!(matches!(
        parameters_error,
        ConverseToolsError::ParametersNotAnObject
    ));
    let missing_type = json(r#"{"properties": {}}"#);
    let type_error = BedrockToolSpec {
        name: "tool",
        description: "d",
        parameters: &missing_type,
        strict: None,
        supports_strict_tools: false,
    }
    .into_block()
    .expect_err("a parameter type is required");
    assert!(matches!(
        type_error,
        ConverseToolsError::MissingParameterType
    ));
    assert_eq!(
        ConverseToolsError::MissingParameterType.status_code(),
        Some(400),
        "a missing type is a bad request"
    );
}

#[test]
fn malformed_request_tool_parameters_answer_400() {
    let parameters = json(r#"{"properties": {}}"#);
    let error = BedrockToolSpec {
        name: "tool",
        description: "d",
        parameters: &parameters,
        strict: None,
        supports_strict_tools: false,
    }
    .into_block()
    .expect_err("parameters without a type are rejected");
    assert!(matches!(&error, ConverseToolsError::MissingParameterType));
    assert_eq!(error.status_code(), Some(400));
}

pub(crate) fn supported_media_types() {
    assert_eq!(SUPPORTED_IMAGE_TYPES, ["png", "jpeg", "gif", "webp"]);
    assert_eq!(
        SUPPORTED_DOCUMENT_TYPES,
        [
            "pdf", "csv", "doc", "docx", "xls", "xlsx", "html", "txt", "md"
        ]
    );
    assert_eq!(
        SUPPORTED_VIDEO_TYPES,
        [
            "mp4", "mov", "mkv", "webm", "flv", "mpeg", "mpg", "wmv", "3gp"
        ]
    );
}
