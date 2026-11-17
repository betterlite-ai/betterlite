use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::NamedCapability;
use indexmap::IndexMap;

use super::support::{
    CATALOG, Fixture, LLAMA, MODEL, NOVA, Predicates, Query, json, object, runtime,
};
use crate::converse_tools::error::ConverseToolsError;
use crate::converse_tools::response_format::{
    ResponseFormatRequest, create_output_config, translate_response_format,
};

struct Call<'call> {
    value: Json,
    model: &'call str,
    thinking: bool,
    stream: &'call str,
}

fn call<'call>(value: Json, model: &'call str, thinking: bool, stream: &'call str) -> Call<'call> {
    Call {
        value,
        model,
        thinking,
        stream,
    }
}

fn translated(
    catalogs: &CatalogSources<'_>,
    call: &Call<'_>,
    predicates: &mut Predicates,
) -> Result<IndexMap<String, Json>, ConverseToolsError> {
    let stream = json(call.stream);
    let request = ResponseFormatRequest {
        model: call.model,
        is_thinking_enabled: call.thinking,
        stream: &stream,
        catalogs,
    };
    let mut params = object("{}");
    runtime().block_on(translate_response_format(
        &call.value,
        &request,
        &mut params,
        predicates,
    ))?;
    Ok(params)
}

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
    let mut native = Predicates::with(&[NamedCapability::NativeStructuredOutput.key()]);
    let case = call(
        json(
            r#"{"type": "json_schema", "json_schema": {"name": "answer", "description": "", "schema": {"type": "object", "properties": {"city": {"type": "string"}}}}}"#,
        ),
        MODEL,
        false,
        "true",
    );
    let params = translated(catalogs, &case, &mut native).expect("the format translates natively");
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
    let mut native = Predicates::with(&[NamedCapability::NativeStructuredOutput.key()]);
    let response_schema = call(
        json(r#"{"type": "json_object", "response_schema": {"type": "object"}}"#),
        MODEL,
        false,
        "true",
    );
    let params = translated(catalogs, &response_schema, &mut native).expect("response_schema wins");
    assert_eq!(
        params,
        object(
            r#"{"outputConfig": {"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {"schema": "{\"type\": \"object\", \"additionalProperties\": false}"}}}}, "json_mode": true}"#
        )
    );
    let null_schema = call(
        json(
            r#"{"type": "json_object", "response_schema": null, "json_schema": {"schema": {"type": "object"}, "name": "ignored"}}"#,
        ),
        MODEL,
        false,
        "true",
    );
    assert_eq!(
        translated(catalogs, &null_schema, &mut native).expect("a null schema sets json mode"),
        object(r#"{"json_mode": true}"#)
    );
    let null_field = call(
        json(r#"{"type": "json_schema", "json_schema": {"schema": null}}"#),
        MODEL,
        false,
        "true",
    );
    assert_eq!(
        translated(catalogs, &null_field, &mut native).expect("a null field sets json mode"),
        object(r#"{"json_mode": true}"#)
    );
}

fn format_shapes(catalogs: &CatalogSources<'_>) {
    let mut native = Predicates::with(&[NamedCapability::NativeStructuredOutput.key()]);
    let stream = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream,
        catalogs,
    };
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
}

fn format_errors(catalogs: &CatalogSources<'_>) {
    let mut native = Predicates::with(&[NamedCapability::NativeStructuredOutput.key()]);
    let rows = [
        (json("5"), "the response_format value is not an object"),
        (
            json(r#"{"json_schema": {}}"#),
            "response_format has no type field",
        ),
        (
            json(r#"{"type": "json_schema", "json_schema": 5}"#),
            "response_format.json_schema is not an object",
        ),
        (
            json(r#"{"type": "json_schema", "json_schema": {}}"#),
            "response_format.json_schema has no schema field",
        ),
        (
            json(r#"{"type": "json_schema", "json_schema": {"schema": {}, "name": 5}}"#),
            "response_format json_schema name is not a string",
        ),
    ];
    for (value, message) in rows {
        let error = translated(catalogs, &call(value, MODEL, false, "true"), &mut native)
            .expect_err("a malformed format is rejected");
        assert_eq!(error.to_string(), message);
    }
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
            Some(&json("{\"type\": \"object\", \"note\": \"\u{e9}\"}")),
            Some(""),
            Some("")
        ),
        json(
            r#"{"textFormat": {"type": "json_schema", "structure": {"jsonSchema": {"schema": "{\"type\": \"object\", \"note\": \"\\u00e9\", \"additionalProperties\": false}", "name": "", "description": ""}}}}"#
        )
    );
}

pub(crate) fn unsupported_response_format() {
    let cost = json(CATALOG);
    let fixture = Fixture::new(&cost);
    let catalogs = fixture.catalogs();
    fallback_gates(&catalogs);
    schemaless_formats(&catalogs);
    existing_tools(&catalogs);
}

fn fallback_schema() -> Json {
    json(
        r#"{"type": "json_schema", "json_schema": {"schema": {"type": "object"}, "description": "why"}}"#,
    )
}

// The synthetic tool plus fake_stream, with tool_choice only where the model and request allow forcing one.
fn fallback_gates(catalogs: &CatalogSources<'_>) {
    let gated = object(
        r#"{"tools": [{"type": "function", "function": {"name": "json_tool_call", "parameters": {"type": "object"}, "description": "why"}}], "fake_stream": true, "json_mode": true}"#,
    );
    let choosable = object(
        r#"{"tools": [{"type": "function", "function": {"name": "json_tool_call", "parameters": {"type": "object"}, "description": "why"}}], "tool_choice": {"tool": {"name": "json_tool_call"}}, "json_mode": true}"#,
    );
    let rows = [
        (call(fallback_schema(), MODEL, true, "true"), true, &gated),
        (call(fallback_schema(), MODEL, false, "true"), false, &gated),
        (call(fallback_schema(), NOVA, false, "true"), true, &gated),
        (call(fallback_schema(), LLAMA, false, "1"), true, &choosable),
    ];
    for (case, tool_choice_supported, expected) in rows {
        let mut predicates = if tool_choice_supported {
            Predicates::with(&[NamedCapability::ToolChoice.key()])
        } else {
            Predicates::default()
        };
        let params = translated(catalogs, &case, &mut predicates).expect("the fallback translates");
        assert_eq!(&params, expected);
    }
}

fn schemaless_formats(catalogs: &CatalogSources<'_>) {
    let mut supported = Predicates::with(&[NamedCapability::ToolChoice.key()]);
    let json_object = call(json(r#"{"type": "json_object"}"#), MODEL, false, "true");
    let builtin = call(json(r#"{"type": "builtin"}"#), MODEL, false, "true");
    assert_eq!(
        translated(catalogs, &json_object, &mut supported).expect("json_object sets json mode"),
        object(r#"{"json_mode": true}"#)
    );
    assert_eq!(
        translated(catalogs, &builtin, &mut supported).expect("an unknown format sets json mode"),
        object(r#"{"json_mode": true}"#)
    );
}

fn existing_tools(catalogs: &CatalogSources<'_>) {
    let mut supported = Predicates::with(&[NamedCapability::ToolChoice.key()]);
    let stream = json("true");
    let request = ResponseFormatRequest {
        model: MODEL,
        is_thinking_enabled: false,
        stream: &stream,
        catalogs,
    };
    let mut existing = object(r#"{"tools": [{"toolSpec": {"name": "kept"}}]}"#);
    runtime()
        .block_on(translate_response_format(
            &fallback_schema(),
            &request,
            &mut existing,
            &mut supported,
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

#[test]
fn malformed_request_response_format_answers_400() {
    let cost = json(CATALOG);
    let fixture = Fixture::new(&cost);
    let catalogs = fixture.catalogs();
    let mut native = Predicates::with(&[NamedCapability::NativeStructuredOutput.key()]);
    let empty_envelope = call(
        json(r#"{"type": "json_schema", "json_schema": {}}"#),
        MODEL,
        false,
        "false",
    );
    let error = translated(&catalogs, &empty_envelope, &mut native)
        .expect_err("a json_schema without a schema is rejected");
    assert!(matches!(&error, ConverseToolsError::MissingSchemaField));
    assert_eq!(error.status_code(), Some(400));
}
