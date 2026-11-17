use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_anthropic::anthropic_capability_catalog::forced_tool::ForcedToolChoiceError;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::NamedCapability;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use indexmap::IndexMap;

use super::support::{Fixture, LLAMA, MODEL, NOVA, Predicates, Query, json, runtime};
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
use crate::converse_tools::response_format::{
    ResponseFormatRequest, create_output_config, translate_response_format,
};
use crate::converse_tools::tool_choice::{ToolChoiceValuesBlock, map_tool_choice_values};
use crate::converse_tools::tool_spec::BedrockToolSpec;

const CATALOG: &str = r#"{
    "anthropic.claude-sonnet-4-5-20250929-v1:0": {"supports_forced_tool_use": true},
    "amazon.nova-pro-v1:0": {"supports_forced_tool_use": false},
    "meta.llama3-3-70b-instruct-v1:0": {"supports_forced_tool_use": true}
}"#;

fn object(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(text) else {
        panic!("the fixture is an object");
    };
    fields
}

fn choice(
    model: &str,
    tool_choice: &str,
    drop: DropParams,
    catalogs: &betterlite_anthropic::anthropic_capability_catalog::CatalogSources<'_>,
) -> Result<Option<ToolChoiceValuesBlock>, ConverseToolsError> {
    let mut names = ToolNameMap::new();
    map_tool_choice_values(model, &json(tool_choice), drop, catalogs, &mut names)
}

pub(crate) fn tool_choice_values() {
    let cost = json(CATALOG);
    let fixture = Fixture::new(&cost);
    let catalogs = fixture.catalogs();
    let no_drop = DropParams::default();
    let drop_global = DropParams {
        global: true,
        call: false,
    };
    let drop_call = DropParams {
        global: false,
        call: true,
    };

    assert_eq!(
        choice(MODEL, r#""auto""#, no_drop, &catalogs),
        Ok(Some(object(r#"{"auto": {}}"#)))
    );
    assert_eq!(
        choice(MODEL, r#""required""#, no_drop, &catalogs),
        Ok(Some(object(r#"{"any": {}}"#)))
    );
    assert_eq!(
        choice(NOVA, r#""required""#, drop_call, &catalogs),
        Ok(Some(object(r#"{"auto": {}}"#)))
    );
    assert_eq!(
        choice(NOVA, r#""required""#, drop_global, &catalogs),
        Ok(Some(object(r#"{"auto": {}}"#)))
    );
    let downgraded = choice(NOVA, r#""required""#, no_drop, &catalogs).expect_err("no drop asked");
    assert!(matches!(
        downgraded,
        ConverseToolsError::ForcedTool(ForcedToolChoiceError::Unsupported { .. })
    ));
    assert_eq!(downgraded.status_code(), Some(400));

    assert_eq!(choice(MODEL, r#""none""#, drop_global, &catalogs), Ok(None));
    assert_eq!(choice(MODEL, r#""none""#, drop_call, &catalogs), Ok(None));
    let kept_none =
        choice(MODEL, r#""none""#, no_drop, &catalogs).expect_err("none is unsupported");
    assert_eq!(kept_none, ConverseToolsError::UnsupportedNone);
    assert_eq!(kept_none.status_code(), Some(400));

    let unknown = choice(MODEL, r#""bogus""#, drop_global, &catalogs).expect_err("not dropped");
    assert_eq!(
        unknown,
        ConverseToolsError::UnsupportedChoice {
            value: "bogus".to_owned()
        }
    );
    assert_eq!(unknown.status_code(), Some(400));
    assert_eq!(
        choice(MODEL, "7", drop_global, &catalogs)
            .expect_err("a number is not a tool choice")
            .status_code(),
        Some(400)
    );
    assert_eq!(
        choice(MODEL, "null", no_drop, &catalogs)
            .expect_err("null is not a tool choice")
            .status_code(),
        Some(400)
    );

    assert_eq!(
        choice(
            MODEL,
            r#"{"type": "function", "function": {"name": "get weather!"}}"#,
            no_drop,
            &catalogs
        ),
        Ok(Some(object(r#"{"tool": {"name": "get_weather_"}}"#)))
    );
    assert_eq!(
        choice(MODEL, r#"{"type": "auto"}"#, no_drop, &catalogs),
        Ok(Some(object(r#"{"tool": {"name": ""}}"#)))
    );
    assert_eq!(
        choice(MODEL, r#"{"function": {"name": null}}"#, no_drop, &catalogs),
        Ok(Some(object(r#"{"tool": {"name": null}}"#)))
    );
    assert_eq!(
        choice(
            NOVA,
            r#"{"function": {"name": "specific"}}"#,
            drop_call,
            &catalogs
        ),
        Ok(Some(object(r#"{"auto": {}}"#)))
    );
    assert_eq!(
        choice(
            NOVA,
            r#"{"function": {"name": "specific"}}"#,
            no_drop,
            &catalogs
        )
        .expect_err("forced tool use is refused")
        .status_code(),
        Some(400)
    );

    assert_eq!(
        choice(MODEL, r#"{"function": 5}"#, no_drop, &catalogs),
        Err(ConverseToolsError::ToolChoiceFunction)
    );
    assert_eq!(
        choice(MODEL, r#"{"function": {"name": 5}}"#, no_drop, &catalogs),
        Err(ConverseToolsError::ToolChoiceName)
    );
    assert_eq!(
        ConverseToolsError::ToolChoiceName.status_code(),
        None,
        "a malformed name is not a 400"
    );
}

pub(crate) fn computer_use_tool_split() {
    let tools = json(
        r#"[
            {"type": "computer_use_preview", "name": "computer"},
            {"type": "computer_20250124", "function": {"name": "comp", "parameters": {"type": "object", "name": "overridden"}}},
            {"type": "bash_20250124"},
            {"type": "text_editor_20250124"},
            {"type": "computer_20250124"},
            {"type": "computer_20250124", "name": null},
            {"type": "custom_tool", "description": "kept"},
            {"name": "nameless"}
        ]"#,
    );
    let Json::List(tools) = tools else {
        panic!("the fixture is a list");
    };

    assert_eq!(is_computer_use_tool_used(None), Ok(false));
    assert_eq!(is_computer_use_tool_used(Some(tools.as_slice())), Ok(true));
    assert_eq!(
        is_computer_use_tool_used(Some(&[json(r#"{"name": "plain"}"#)])),
        Ok(false)
    );
    assert_eq!(
        is_computer_use_tool_used(Some(&[json(r#"{"type": "custom_tool"}"#)])),
        Ok(false)
    );

    let (computer, regular) =
        separate_computer_use_tools(&tools).expect("the tools separate cleanly");
    assert_eq!(computer.len(), 6);
    assert_eq!(regular.len(), 2);
    assert_eq!(computer.first(), tools.first());
    assert_eq!(
        regular,
        vec![
            json(r#"{"type": "custom_tool", "description": "kept"}"#),
            json(r#"{"name": "nameless"}"#)
        ]
    );

    let transformed = transform_computer_use_tools(&tools).expect("the tools transform cleanly");
    assert_eq!(
        transformed,
        vec![
            json(r#"{"type": "computer_use_preview", "name": "computer"}"#),
            json(r#"{"type": "object", "name": "overridden"}"#),
            json(r#"{"type": "bash_20250124", "name": "bash"}"#),
            json(r#"{"type": "text_editor_20250124", "name": "str_replace_editor"}"#),
            json(r#"{"type": "computer_20250124"}"#),
            json(r#"{"type": "computer_20250124", "name": null}"#),
            json(r#"{"type": "custom_tool", "description": "kept"}"#),
            json(r#"{"name": "nameless"}"#),
        ]
    );
    assert_eq!(
        transform_computer_use_tools(&[json(r#"{"type": "computer_x", "function": {}}"#)]),
        Ok(vec![json(r#"{"type": "computer_x", "name": "computer"}"#)])
    );

    assert_eq!(
        is_computer_use_tool_used(Some(&[json("5")])),
        Err(ConverseToolsError::ToolNotAnObject)
    );
    assert_eq!(
        is_computer_use_tool_used(Some(&[json(r#"{"type": 5}"#)])),
        Err(ConverseToolsError::ToolType)
    );
    assert_eq!(
        transform_computer_use_tools(&[json(r#"{"type": "computer_x", "function": 5}"#)]),
        Err(ConverseToolsError::ToolFunction)
    );
    assert_eq!(
        transform_computer_use_tools(&[json(
            r#"{"type": "computer_x", "function": {"parameters": null}}"#
        )]),
        Err(ConverseToolsError::ToolParameters)
    );
    assert_eq!(
        transform_computer_use_tools(&[json(r#"{"type": 5}"#)]),
        Err(ConverseToolsError::ToolType)
    );
}

pub(crate) fn json_mode_tool() {
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
        json(r#"{"type": "function", "function": {"name": "json_tool_call", "parameters": {}}}"#)
    );
    assert_eq!(
        create_json_tool_call(Some(&json("{}")), Some("why")),
        json(
            r#"{"type": "function", "function": {"name": "json_tool_call", "parameters": {}, "description": "why"}}"#
        )
    );

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
    assert_eq!(
        append_tools(&mut object(r#"{"tools": 5}"#), vec![tool.clone()]),
        Err(ConverseToolsError::ToolsNotList)
    );

    let mut streamed = object("{}");
    apply_tool_call_transformation(&[tool.clone()], LLAMA, &json("true"), &mut streamed)
        .expect("the tools apply");
    assert_eq!(streamed.get("fake_stream"), Some(&Json::Bool(true)));
    let mut prefixed = object("{}");
    apply_tool_call_transformation(
        &[tool.clone()],
        "us.meta.llama3-3-70b-instruct-v1:0",
        &json("true"),
        &mut prefixed,
    )
    .expect("the tools apply");
    assert_eq!(prefixed.get("fake_stream"), Some(&Json::Bool(true)));
    for stream in ["false", "null", "1", r#""true""#] {
        let mut params = object("{}");
        apply_tool_call_transformation(&[tool.clone()], LLAMA, &json(stream), &mut params)
            .expect("the tools apply");
        assert_eq!(
            params.get("fake_stream"),
            None,
            "stream {stream} is not true"
        );
    }
    let mut other = object("{}");
    apply_tool_call_transformation(&[tool], "amazon.nova-pro-v1:0", &json("true"), &mut other)
        .expect("the tools apply");
    assert_eq!(other.get("fake_stream"), None);
}

pub(crate) fn native_structured_output() {
    let cost = json(CATALOG);
    let fixture = Fixture::new(&cost);
    let catalogs = fixture.catalogs();
    let stream_true = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs: &catalogs,
    };
    let mut native = Predicates::with(&[NamedCapability::NativeStructuredOutput.key()]);
    let mut params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &json(
                r#"{"type": "json_schema", "json_schema": {"name": "answer", "description": "", "schema": {"type": "object", "properties": {"city": {"type": "string"}}}}}"#,
            ),
            &request,
            &mut params,
            &mut native,
        ))
        .expect("the format translates natively");
    assert_eq!(
        params,
        object(
            r#"{"outputConfig": {"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {"schema": "{\"type\": \"object\", \"properties\": {\"city\": {\"type\": \"string\"}}, \"additionalProperties\": false}", "name": "answer", "description": ""}}}}, "json_mode": true}"#
        )
    );
    assert_eq!(
        native.queries,
        vec![Query {
            model: MODEL.to_owned(),
            provider: Some("bedrock_converse".to_owned()),
            key: "supports_native_structured_output".to_owned(),
        }]
    );

    let mut no_name = object("{}");
    runtime()
        .block_on(translate_response_format(
            &json(r#"{"type": "json_object", "response_schema": {"type": "object"}}"#),
            &request,
            &mut no_name,
            &mut native,
        ))
        .expect("response_schema wins");
    assert_eq!(
        no_name,
        object(
            r#"{"outputConfig": {"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {"schema": "{\"type\": \"object\", \"additionalProperties\": false}"}}}}, "json_mode": true}"#
        )
    );

    let mut null_schema = object("{}");
    runtime()
        .block_on(translate_response_format(
            &json(
                r#"{"type": "json_object", "response_schema": null, "json_schema": {"schema": {"type": "object"}, "name": "ignored"}}"#,
            ),
            &request,
            &mut null_schema,
            &mut native,
        ))
        .expect("a null response_schema wins");
    assert_eq!(null_schema, object(r#"{"json_mode": true}"#));

    let mut text = object(r#"{"existing": 1}"#);
    runtime()
        .block_on(translate_response_format(
            &json(r#"{"type": "text", "json_schema": {"schema": {}}}"#),
            &request,
            &mut text,
            &mut native,
        ))
        .expect("text is a no-op");
    assert_eq!(text, object(r#"{"existing": 1}"#));

    assert_eq!(
        runtime().block_on(translate_response_format(
            &json("5"),
            &request,
            &mut object("{}"),
            &mut native,
        )),
        Err(ConverseToolsError::FormatNotAnObject)
    );
    assert_eq!(
        runtime().block_on(translate_response_format(
            &json(r#"{"json_schema": {}}"#),
            &request,
            &mut object("{}"),
            &mut native,
        )),
        Err(ConverseToolsError::MissingFormatType)
    );
    assert_eq!(
        runtime().block_on(translate_response_format(
            &json(r#"{"type": "json_schema", "json_schema": 5}"#),
            &request,
            &mut object("{}"),
            &mut native,
        )),
        Err(ConverseToolsError::JsonSchemaNotAnObject)
    );
    assert_eq!(
        runtime().block_on(translate_response_format(
            &json(r#"{"type": "json_schema", "json_schema": {}}"#),
            &request,
            &mut object("{}"),
            &mut native,
        )),
        Err(ConverseToolsError::MissingSchemaField)
    );
    assert_eq!(
        runtime().block_on(translate_response_format(
            &json(r#"{"type": "json_schema", "json_schema": {"schema": {}, "name": 5}}"#),
            &request,
            &mut object("{}"),
            &mut native,
        )),
        Err(ConverseToolsError::NotText { field: "name" })
    );

    assert_eq!(
        create_output_config(None, None, None),
        json(
            r#"{"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {"schema": "{}"}}}}"#
        )
    );
    assert_eq!(
        create_output_config(Some(&json(r#"{"note": "café"}"#)), Some(""), Some("")),
        json(
            r#"{"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {"schema": "{\"note\": \"caf\\u00e9\", \"additionalProperties\": false}", "name": "", "description": ""}}}}"#
        )
    );
}

pub(crate) fn additional_properties_recursion() {
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

    let mut deep = Json::Object(object(r#"{"type": "object"}"#));
    for _ in 0_usize..2000 {
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
    assert_eq!(depth, 2000, "every level of the deep schema is patched");

    let source = json(r#"{"type": "object", "properties": {"a": {"type": "object"}}}"#);
    let patched = add_additional_properties(&source);
    assert_eq!(
        source,
        json(r#"{"type": "object", "properties": {"a": {"type": "object"}}}"#),
        "the source schema is not mutated"
    );
    assert_ne!(patched, source);
}

pub(crate) fn unsupported_response_format() {
    let cost = json(CATALOG);
    let fixture = Fixture::new(&cost);
    let catalogs = fixture.catalogs();
    let stream_true = json("true");
    let stream_number = json("1");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs: &catalogs,
    };
    let schema = json(
        r#"{"type": "json_schema", "json_schema": {"schema": {"type": "object"}, "description": "why"}}"#,
    );
    let mut tool_choice_supported = Predicates::with(&[NamedCapability::ToolChoice.key()]);
    let mut params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &schema,
            &request,
            &mut params,
            &mut tool_choice_supported,
        ))
        .expect("the fallback translates");
    assert_eq!(
        params,
        object(
            r#"{"tools": [{"type": "function", "function": {"name": "json_tool_call", "parameters": {"type": "object"}, "description": "why"}}], "tool_choice": {"tool": {"name": "json_tool_call"}}, "fake_stream": true, "json_mode": true}"#
        )
    );

    let thinking = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: true,
        stream: &stream_true,
        catalogs: &catalogs,
    };
    let mut thinking_params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &schema,
            &thinking,
            &mut thinking_params,
            &mut tool_choice_supported,
        ))
        .expect("the fallback translates");
    assert!(thinking_params.get("tools").is_some());
    assert_eq!(thinking_params.get("tool_choice"), None);
    assert_eq!(thinking_params.get("fake_stream"), Some(&Json::Bool(true)));

    let mut no_tool_choice = Predicates::default();
    let mut plain_params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &schema,
            &request,
            &mut plain_params,
            &mut no_tool_choice,
        ))
        .expect("the fallback translates");
    assert_eq!(plain_params.get("tool_choice"), None);
    assert_eq!(plain_params.get("fake_stream"), Some(&Json::Bool(true)));

    let nova = ResponseFormatRequest {
        model: NOVA,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs: &catalogs,
    };
    let mut nova_params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &schema,
            &nova,
            &mut nova_params,
            &mut tool_choice_supported,
        ))
        .expect("the fallback translates");
    assert_eq!(
        nova_params.get("tool_choice"),
        None,
        "forced use is refused"
    );
    assert_eq!(nova_params.get("fake_stream"), Some(&Json::Bool(true)));

    let mut no_fake_stream = object("{}");
    let number_stream = ResponseFormatRequest {
        model: LLAMA,
        is_thinking_enabled: false,
        stream: &stream_number,
        catalogs: &catalogs,
    };
    runtime()
        .block_on(translate_response_format(
            &schema,
            &number_stream,
            &mut no_fake_stream,
            &mut tool_choice_supported,
        ))
        .expect("the fallback translates");
    assert_eq!(no_fake_stream.get("fake_stream"), None);

    let mut schemaless = object("{}");
    runtime()
        .block_on(translate_response_format(
            &json(r#"{"type": "json_object"}"#),
            &request,
            &mut schemaless,
            &mut tool_choice_supported,
        ))
        .expect("json_object needs no tool");
    assert_eq!(schemaless, object(r#"{"json_mode": true}"#));

    let mut unknown = object("{}");
    runtime()
        .block_on(translate_response_format(
            &json(r#"{"type": "builtin"}"#),
            &request,
            &mut unknown,
            &mut tool_choice_supported,
        ))
        .expect("an unknown type is not rejected here");
    assert_eq!(unknown, object(r#"{"json_mode": true}"#));

    let mut existing = object(r#"{"tools": [{"toolSpec": {"name": "kept"}}]}"#);
    runtime()
        .block_on(translate_response_format(
            &schema,
            &request,
            &mut existing,
            &mut tool_choice_supported,
        ))
        .expect("the fallback translates");
    let Json::List(tools) = existing.get("tools").expect("tools exist") else {
        panic!("tools stay a list");
    };
    assert_eq!(tools.len(), 2);
    assert_eq!(
        tools.first(),
        Some(&json(r#"{"toolSpec": {"name": "kept"}}"#))
    );
}

pub(crate) fn strict_tool_spec() {
    let parameters = json(r#"{"type": "object", "properties": {"a": 1}, "required": ["a"]}"#);
    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "desc",
            parameters: &parameters,
            strict: None,
            supports_strict_tools: false,
        }
        .into_block(),
        Ok(json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {"a": 1}, "required": ["a"]}}, "name": "tool", "description": "desc"}}"#
        ))
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
        .into_block(),
        Ok(json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "tool", "description": ""}}"#
        ))
    );

    let nulls = json(r#"{"type": null, "properties": null, "required": null}"#);
    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "d",
            parameters: &nulls,
            strict: None,
            supports_strict_tools: false,
        }
        .into_block(),
        Ok(json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": null, "properties": null, "required": null}}, "name": "tool", "description": "d"}}"#
        ))
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
        .into_block(),
        Ok(json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "tool", "description": "d"}}"#
        ))
    );

    let strict_parameters =
        json(r#"{"type": "object", "additionalProperties": false, "properties": {}}"#);
    for strict in [Some(true), Some(false), None] {
        let block = BedrockToolSpec {
            name: "tool",
            description: "d",
            parameters: &strict_parameters,
            strict,
            supports_strict_tools: true,
        }
        .into_block()
        .expect("the strict spec builds");
        let Json::Object(fields) = &block else {
            panic!("the block is an object");
        };
        let Json::Object(spec) = fields.get("toolSpec").expect("a tool spec") else {
            panic!("the spec is an object");
        };
        let expected_strict = if strict == Some(true) {
            Some(Json::Bool(true))
        } else {
            None
        };
        assert_eq!(spec.get("strict"), expected_strict.as_ref(), "{strict:?}");
        let Json::Object(schema) = spec.get("inputSchema").expect("an input schema") else {
            panic!("the input schema is an object");
        };
        let Json::Object(json_schema) = schema.get("json").expect("a json schema") else {
            panic!("the json schema is an object");
        };
        assert_eq!(
            json_schema.get("additionalProperties"),
            Some(&Json::Bool(false))
        );
    }

    let null_additional = json(r#"{"type": "object", "additionalProperties": null}"#);
    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "d",
            parameters: &null_additional,
            strict: Some(true),
            supports_strict_tools: true,
        }
        .into_block(),
        Ok(json(
            r#"{"toolSpec": {"inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}, "name": "tool", "description": "d", "strict": true}}"#
        ))
    );

    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "d",
            parameters: &json("5"),
            strict: None,
            supports_strict_tools: false,
        }
        .into_block(),
        Err(ConverseToolsError::ParametersNotAnObject)
    );
    assert_eq!(
        BedrockToolSpec {
            name: "tool",
            description: "d",
            parameters: &json(r#"{"properties": {}}"#),
            strict: None,
            supports_strict_tools: false,
        }
        .into_block(),
        Err(ConverseToolsError::MissingParameterType)
    );
    assert_eq!(
        ConverseToolsError::MissingParameterType.status_code(),
        None,
        "a missing type is not a 400"
    );
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
