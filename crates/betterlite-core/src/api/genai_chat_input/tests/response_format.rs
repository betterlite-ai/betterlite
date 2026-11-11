use betterlite_openai::chat::ChatRequest;
use rstest::rstest;
use serde_json::{Value, json};

use super::prelude::{limits_with_nodes, translate, translate_result, translate_with};
use crate::api::genai_chat_input::GenAiInputError;
use crate::api::genai_chat_input::input::GenerateLiteLLMParams;

fn body(config: &Value) -> Value {
    json!({"model": "m", "contents": [], "generationConfig": config})
}

fn schema_of(request: &ChatRequest) -> Value {
    let response_format = request.response_format.as_ref().expect("a response format");
    assert_eq!(response_format.kind, "json_schema");
    let json_schema = response_format.json_schema.as_ref().expect("json_schema");
    assert_eq!(json_schema["name"], "response");
    json_schema["schema"].clone()
}

#[test]
fn an_object_schema_becomes_a_named_json_schema() {
    let request = translate(body(&json!({
        "responseMimeType": "application/json",
        "responseSchema": {"type": "object", "properties": {"name": {"type": "string"}}},
    })));
    assert_eq!(
        schema_of(&request),
        json!({"type": "object", "properties": {"name": {"type": "string"}}})
    );
}

#[test]
fn a_json_schema_without_a_mime_type_is_kept() {
    let request = translate(body(&json!({
        "responseJsonSchema": {"type": "object", "properties": {}},
    })));
    assert_eq!(
        schema_of(&request),
        json!({"type": "object", "properties": {}})
    );
}

#[rstest]
#[case::text("text/plain")]
#[case::absent_mime_and_schema("application/xml")]
#[case::empty("")]
fn a_non_json_mime_type_drops_the_response_format(#[case] mime: &str) {
    let request = translate(body(&json!({
        "responseMimeType": mime,
        "responseSchema": {"type": "object", "properties": {}},
    })));
    assert_eq!(request.response_format, None);
}

#[test]
fn a_schema_without_a_type_is_dropped() {
    let request = translate(body(&json!({"responseSchema": {"properties": {}}})));
    assert_eq!(request.response_format, None);
}

#[test]
fn a_schema_whose_type_is_not_object_is_dropped() {
    let request = translate(body(&json!({"responseSchema": {"type": "string"}})));
    assert_eq!(request.response_format, None);
}

#[rstest]
#[case::string(json!("schema"))]
#[case::array(json!([{"type": "object"}]))]
#[case::null(Value::Null)]
fn a_non_object_schema_is_dropped(#[case] schema: Value) {
    let request = translate(body(&json!({"responseSchema": schema})));
    assert_eq!(request.response_format, None);
}

#[test]
fn missing_schema_drops_the_response_format() {
    let request = translate(body(&json!({"responseMimeType": "application/json"})));
    assert_eq!(request.response_format, None);
}

#[test]
fn an_upper_case_object_type_is_normalized_and_kept() {
    let request = translate(body(&json!({
        "responseSchema": {"type": "OBJECT", "properties": {"n": {"type": "NUMBER"}}},
    })));
    assert_eq!(
        schema_of(&request),
        json!({"type": "object", "properties": {"n": {"type": "number"}}})
    );
}

#[test]
fn the_gemini_only_keys_are_removed_from_the_whole_schema() {
    let request = translate(body(&json!({
        "responseSchema": {
            "type": "object",
            "propertyOrdering": ["a"],
            "property_ordering": ["a"],
            "properties": {
                "a": {"type": "object", "propertyOrdering": ["b"], "properties": {"b": {"type": "string"}}},
            },
            "items": [{"type": "object", "property_ordering": []}],
        },
    })));
    assert_eq!(
        schema_of(&request),
        json!({
            "type": "object",
            "properties": {"a": {"type": "object", "properties": {"b": {"type": "string"}}}},
            "items": [{"type": "object"}],
        })
    );
}

#[test]
fn the_snake_case_mime_and_schema_keys_are_first_present_fallbacks() {
    let request = translate(body(&json!({
        "response_mime_type": "application/json",
        "response_schema": {"type": "object", "properties": {"snake": {"type": "boolean"}}},
    })));
    assert_eq!(
        schema_of(&request),
        json!({"type": "object", "properties": {"snake": {"type": "boolean"}}})
    );
}

#[test]
fn the_json_schema_key_wins_over_the_schema_key() {
    let request = translate(body(&json!({
        "responseSchema": {"type": "object", "chosen": "schema"},
        "responseJsonSchema": {"type": "object", "chosen": "json_schema"},
    })));
    assert_eq!(schema_of(&request)["chosen"], "json_schema");
}

fn provider_params(provider: Option<&str>) -> GenerateLiteLLMParams {
    GenerateLiteLLMParams {
        api_base: None,
        api_key: None,
        api_version: None,
        custom_llm_provider: provider.map(str::to_owned),
        max_retries: None,
        organization: None,
    }
}

#[rstest]
#[case::gpt_4("gpt-4", None, false)]
#[case::gpt_3_5_16k("gpt-3.5-turbo-16k", None, false)]
#[case::gpt_4o("gpt-4o", None, true)]
#[case::custom_anthropic("gpt-4", Some("anthropic"), true)]
#[case::empty_provider("gpt-4", Some(""), false)]
#[case::openai_provider("gpt-4", Some("openai"), false)]
#[case::azure_prefix("azure/gpt-4", None, true)]
#[case::openai_prefix("openai/gpt-4", None, false)]
#[case::unknown_prefix("my-org/gpt-4", None, true)]
#[case::o_series_response_schema("o3", None, true)]
fn the_provider_response_format_support_decides_the_field(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] supported: bool,
) {
    let (request, _) = translate_with(
        json!({
            "model": model,
            "contents": [],
            "generationConfig": {"responseSchema": {"type": "object", "properties": {}}},
        }),
        &provider_params(provider),
    );
    assert_eq!(request.response_format.is_some(), supported);
}

fn nested_schema(depth: usize, total: usize, marked: &[(usize, &str)], via: &str) -> Value {
    let mut object = json!({"type": "object"});
    if let Some((_, key)) = marked.iter().find(|(at, _)| *at == depth) {
        object[*key] = json!(["x"]);
    }
    if depth < total {
        let child = nested_schema(depth.saturating_add(1), total, marked, via);
        if via == "items" {
            object["items"] = child;
        } else {
            object["anyOf"] = json!([child]);
        }
    }
    object
}

fn descend<'call>(schema: &'call Value, depth: usize, via: &str) -> &'call Value {
    if depth == 0 {
        return schema;
    }
    if via == "items" {
        descend(&schema["items"], depth.saturating_sub(1), via)
    } else {
        descend(&schema["anyOf"][0], depth.saturating_sub(1), via)
    }
}

#[test]
fn the_gemini_key_filter_stops_past_litellms_recursion_bound() {
    let schema = nested_schema(
        0,
        101,
        &[(100, "propertyOrdering"), (101, "property_ordering")],
        "items",
    );
    let request = translate(body(&json!({"responseSchema": schema})));
    let cleaned = schema_of(&request);
    assert!(
        descend(&cleaned, 100, "items")
            .get("propertyOrdering")
            .is_none()
    );
    assert!(
        descend(&cleaned, 101, "items")
            .get("property_ordering")
            .is_some()
    );
}

#[test]
fn a_response_schema_past_the_limits_fails_the_translation() {
    let limits = limits_with_nodes(3);
    let error = translate_result(
        body(&json!({"responseSchema": {
            "type": "object",
            "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
        }})),
        None,
        limits,
    )
    .expect_err("a schema limit error");
    assert!(matches!(error, GenAiInputError::Schema(_)));
}

#[test]
fn the_gemini_key_filter_counts_a_list_as_no_extra_level() {
    let schema = nested_schema(
        0,
        101,
        &[(100, "propertyOrdering"), (101, "property_ordering")],
        "anyOf",
    );
    let request = translate(body(&json!({"responseSchema": schema})));
    let cleaned = schema_of(&request);
    assert!(
        descend(&cleaned, 100, "anyOf")
            .get("propertyOrdering")
            .is_none()
    );
    assert!(
        descend(&cleaned, 101, "anyOf")
            .get("property_ordering")
            .is_some()
    );
}

#[test]
fn the_camel_case_response_keys_win_over_the_snake_case_ones() {
    let request = translate(body(&json!({
        "responseMimeType": "application/json",
        "response_mime_type": "text/plain",
        "responseJsonSchema": {"type": "object", "chosen": "camel"},
        "response_json_schema": {"type": "object", "chosen": "snake"},
    })));
    assert_eq!(schema_of(&request)["chosen"], "camel");
}

#[test]
fn a_camel_case_non_json_mime_type_drops_the_snake_case_schema() {
    let request = translate(body(&json!({
        "responseMimeType": "text/plain",
        "response_mime_type": "application/json",
        "responseSchema": {"type": "object", "properties": {}},
    })));
    assert_eq!(request.response_format, None);
}
