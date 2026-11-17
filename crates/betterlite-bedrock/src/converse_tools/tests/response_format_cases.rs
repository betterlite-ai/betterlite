use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::NamedCapability;

use super::support::{
    CATALOG, Fixture, LLAMA, MODEL, NOVA, Predicates, Query, json, object, runtime,
};
use crate::converse_tools::error::ConverseToolsError;
use crate::converse_tools::response_format::{
    ResponseFormatRequest, create_output_config, translate_response_format,
};

pub(crate) fn native_structured_output() {
    let cost = json(CATALOG);
    let fixture = Fixture::new(&cost);
    let catalogs = fixture.catalogs();
    native_config(&catalogs);
    response_schema_wins(&catalogs);
    format_shapes(&catalogs);
    format_errors(&catalogs);
    output_config_blocks();
}

fn native_config(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs,
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
}

fn response_schema_wins(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs,
    };
    let mut native = Predicates::with(&[NamedCapability::NativeStructuredOutput.key()]);
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
}

fn format_shapes(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs,
    };
    let mut native = Predicates::with(&[NamedCapability::NativeStructuredOutput.key()]);
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
}

fn format_errors(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs,
    };
    let mut native = Predicates::with(&[NamedCapability::NativeStructuredOutput.key()]);
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
}

fn output_config_blocks() {
    assert_eq!(
        create_output_config(None, None, None),
        json(
            r#"{"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {"schema": "{}"}}}}"#
        )
    );
    assert_eq!(
        create_output_config(
            Some(&json(r#"{"type": "object", "note": "café"}"#)),
            Some(""),
            Some("")
        ),
        json(
            r#"{"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {"schema": "{\"type\": \"object\", \"note\": \"caf\\u00e9\", \"additionalProperties\": false}", "name": "", "description": ""}}}}"#
        )
    );
}

pub(crate) fn unsupported_response_format() {
    let cost = json(CATALOG);
    let fixture = Fixture::new(&cost);
    let catalogs = fixture.catalogs();
    fallback_translation(&catalogs);
    thinking_gate(&catalogs);
    capability_gate(&catalogs);
    forced_gate(&catalogs);
    stream_literals(&catalogs);
    schemaless_formats(&catalogs);
    existing_tools(&catalogs);
}

fn fallback_schema() -> Json {
    json(
        r#"{"type": "json_schema", "json_schema": {"schema": {"type": "object"}, "description": "why"}}"#,
    )
}

fn fallback_translation(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs,
    };
    let mut tool_choice_supported = Predicates::with(&[NamedCapability::ToolChoice.key()]);
    let mut params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &fallback_schema(),
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
}

fn thinking_gate(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let thinking = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: true,
        stream: &stream_true,
        catalogs,
    };
    let mut tool_choice_supported = Predicates::with(&[NamedCapability::ToolChoice.key()]);
    let mut params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &fallback_schema(),
            &thinking,
            &mut params,
            &mut tool_choice_supported,
        ))
        .expect("the fallback translates");
    assert!(params.get("tools").is_some());
    assert_eq!(params.get("tool_choice"), None);
    assert_eq!(params.get("fake_stream"), Some(&Json::Bool(true)));
}

fn capability_gate(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs,
    };
    let mut no_tool_choice = Predicates::default();
    let mut params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &fallback_schema(),
            &request,
            &mut params,
            &mut no_tool_choice,
        ))
        .expect("the fallback translates");
    assert_eq!(params.get("tool_choice"), None);
    assert_eq!(params.get("fake_stream"), Some(&Json::Bool(true)));
}

fn forced_gate(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let nova = ResponseFormatRequest {
        model: NOVA,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs,
    };
    let mut tool_choice_supported = Predicates::with(&[NamedCapability::ToolChoice.key()]);
    let mut params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &fallback_schema(),
            &nova,
            &mut params,
            &mut tool_choice_supported,
        ))
        .expect("the fallback translates");
    assert_eq!(params.get("tool_choice"), None, "forced use is refused");
    assert_eq!(params.get("fake_stream"), Some(&Json::Bool(true)));
}

fn stream_literals(catalogs: &CatalogSources<'_>) {
    let number_stream = json("1");
    let request = ResponseFormatRequest {
        model: LLAMA,
        is_thinking_enabled: false,
        stream: &number_stream,
        catalogs,
    };
    let mut tool_choice_supported = Predicates::with(&[NamedCapability::ToolChoice.key()]);
    let mut params = object("{}");
    runtime()
        .block_on(translate_response_format(
            &fallback_schema(),
            &request,
            &mut params,
            &mut tool_choice_supported,
        ))
        .expect("the fallback translates");
    assert_eq!(params.get("fake_stream"), None);
}

fn schemaless_formats(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs,
    };
    let mut tool_choice_supported = Predicates::with(&[NamedCapability::ToolChoice.key()]);
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
}

fn existing_tools(catalogs: &CatalogSources<'_>) {
    let stream_true = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream_true,
        catalogs,
    };
    let mut tool_choice_supported = Predicates::with(&[NamedCapability::ToolChoice.key()]);
    let mut existing = object(r#"{"tools": [{"toolSpec": {"name": "kept"}}]}"#);
    runtime()
        .block_on(translate_response_format(
            &fallback_schema(),
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
