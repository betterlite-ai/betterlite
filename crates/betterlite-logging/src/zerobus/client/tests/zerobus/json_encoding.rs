use betterlite_models::decisions::Json;

use super::support::{expected, row};

const NESTED_PAYLOAD: &str = r#"{
    "request_tags": ["prod"],
    "messages": [{"role": "user", "content": "hi"}],
    "response": {"choices": [{"message": {"role": "assistant", "content": "hello"}}]},
    "model_parameters": {"temperature": 0.2},
    "hidden_params": {"response_cost": 0.0015},
    "guardrail_information": [{"guardrail_name": "pii"}],
    "error_information": {"error_code": "400"},
    "cost_breakdown": {"input_cost": 0.001, "output_cost": 0.0005}
}"#;

fn encoded(value: Option<String>) -> Json {
    expected(&value.unwrap())
}

#[test]
fn the_nested_fields_are_json_text_for_the_variant_columns() {
    let row = row(NESTED_PAYLOAD);

    assert_eq!(encoded(row.request_tags), expected(r#"["prod"]"#));
    assert_eq!(
        encoded(row.messages),
        expected(r#"[{"role": "user", "content": "hi"}]"#)
    );
    assert_eq!(
        encoded(row.response),
        expected(r#"{"choices": [{"message": {"role": "assistant", "content": "hello"}}]}"#)
    );
    assert_eq!(
        encoded(row.model_parameters),
        expected(r#"{"temperature": 0.2}"#)
    );
    assert_eq!(
        encoded(row.hidden_params),
        expected(r#"{"response_cost": 0.0015}"#)
    );
    assert_eq!(
        encoded(row.guardrail_information),
        expected(r#"[{"guardrail_name": "pii"}]"#)
    );
    assert_eq!(
        encoded(row.error_information),
        expected(r#"{"error_code": "400"}"#)
    );
    assert_eq!(
        encoded(row.cost_breakdown),
        expected(r#"{"input_cost": 0.001, "output_cost": 0.0005}"#)
    );
}

#[test]
fn a_null_nested_field_is_null() {
    let row = row(r#"{
            "messages": null,
            "response": null,
            "request_tags": null,
            "error_information": null,
            "guardrail_information": null,
            "cost_breakdown": null
        }"#);

    assert_eq!(row.messages, None);
    assert_eq!(row.response, None);
    assert_eq!(row.request_tags, None);
    assert_eq!(row.error_information, None);
    assert_eq!(row.guardrail_information, None);
    assert_eq!(row.cost_breakdown, None);
}

#[test]
fn a_missing_nested_field_is_null() {
    let row = row("{}");

    assert_eq!(row.messages, None);
    assert_eq!(row.response, None);
    assert_eq!(row.metadata, None);
    assert_eq!(row.model_parameters, None);
    assert_eq!(row.hidden_params, None);
}

#[test]
fn a_scalar_in_a_variant_column_is_json_encoded() {
    let row = row(r#"{"messages": "hi", "request_tags": 5}"#);

    assert_eq!(encoded(row.messages), expected(r#""hi""#));
    assert_eq!(encoded(row.request_tags), expected("5"));
}

#[test]
fn nul_bytes_are_stripped_from_the_json_text() {
    let row = row(r#"{"messages": {"text": "a\u0000b"}}"#);

    let text = row.messages.unwrap();
    assert!(!text.contains('\u{0}'));
    assert_eq!(expected(&text), expected(r#"{"text": "ab"}"#));
}
