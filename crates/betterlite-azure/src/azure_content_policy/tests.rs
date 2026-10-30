#![cfg(test)]

use super::{ContentPolicyInput, JsonValue, content_policy_error, extract_azure_error};
use betterlite_error::error::ErrorClass;
use rstest::rstest;

fn value(source: &str) -> JsonValue {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::azure_error_body_shape(value(r#"{"error":{"message":"nested"}}"#), Some("nested"))]
#[case::azure_inner_error_precedence(
    value(r#"{"error":{"inner_error":{},"innererror":"chosen"},"innererror":"outer"}"#),
    Some("chosen")
)]
#[case::azure_policy_provider_fields(
    value(r#"{"message":"provider","type":"","code":null}"#),
    Some("provider")
)]
#[case::azure_policy_extensions(
    value(r#"{"error":{"innererror":{"reason":"x"},"param":"drop"}}"#),
    None
)]
#[case::azure_policy_not_returned(JsonValue::Null, None)]
fn azure_content_policy_cases(#[case] body: JsonValue, #[case] expected: Option<&str>) {
    let (selected, _) = extract_azure_error(&body);
    if expected == Some("chosen") {
        assert_eq!(extract_azure_error(&body).1, Some(value(r#""chosen""#)));
    } else if expected == Some("provider") {
        let input = ContentPolicyInput {
            message: "caller".to_owned(),
            model: "gpt".to_owned(),
            extra_information: "debug".to_owned(),
            body,
            response: None,
        };
        let error = content_policy_error(&input);
        assert_eq!(error.body["type"], value(r#""invalid_request_error""#));
        assert_eq!(error.body["code"], value(r#""content_policy_violation""#));
    } else if expected.is_none() && body == JsonValue::Null {
        let input = ContentPolicyInput {
            message: "caller".to_owned(),
            model: "gpt".to_owned(),
            extra_information: "debug".to_owned(),
            body,
            response: None,
        };
        assert_eq!(content_policy_error(&input).status, 400);
    } else if expected.is_none() {
        let input = ContentPolicyInput {
            message: "caller".to_owned(),
            model: "gpt".to_owned(),
            extra_information: "debug".to_owned(),
            body,
            response: None,
        };
        let error = content_policy_error(&input);
        assert_eq!(
            error.provider_specific_fields["inner_error"],
            value(r#"{"reason":"x"}"#)
        );
        assert_eq!(
            error.provider_specific_fields["azure_error"],
            value(r#"{"innererror":{"reason":"x"},"param":"drop"}"#)
        );
    } else {
        assert_eq!(
            selected.get("message").cloned(),
            expected.map(|value| JsonValue::String(value.to_owned()))
        );
    }
}

#[test]
fn azure_policy_empty_array_message_falls_back() {
    let input = ContentPolicyInput {
        message: "caller".to_owned(),
        model: "gpt".to_owned(),
        extra_information: "debug".to_owned(),
        body: value(r#"{"message":[]}"#),
        response: None,
    };
    let error = content_policy_error(&input);
    assert!(error.message.ends_with("caller"));
}

#[test]
fn azure_policy_provider_non_string_message_is_rendered() {
    let input = ContentPolicyInput {
        message: "caller".to_owned(),
        model: "gpt".to_owned(),
        extra_information: "debug".to_owned(),
        body: value(r#"{"message":7}"#),
        response: None,
    };
    let error = content_policy_error(&input);
    assert_eq!(
        error.message,
        "litellm.BadRequestError: litellm.ContentPolicyViolationError: 7"
    );
}

#[rstest]
#[case::bool(r#"{"message":true}"#, "True")]
#[case::array(r#"{"message":[true,null,"text"]}"#, "[True, None, 'text']")]
#[case::object(
    r#"{"message":{"enabled":false,"text":"value"}}"#,
    "{'enabled': False, 'text': 'value'}"
)]
#[case::object_insertion_order(
    r#"{"message":{"text":"v","enabled":false}}"#,
    "{'text': 'v', 'enabled': False}"
)]
#[case::nested_insertion_order(
    r#"{"error":{"message":{"text":{"z":true,"a":null},"enabled":false}}}"#,
    "{'text': {'z': True, 'a': None}, 'enabled': False}"
)]
#[case::array_object_insertion_order(
    r#"{"message":[{"text":"v","enabled":false}]}"#,
    "[{'text': 'v', 'enabled': False}]"
)]
fn azure_policy_provider_message_uses_python_repr(#[case] body: &str, #[case] rendered: &str) {
    let input = ContentPolicyInput {
        message: "caller".to_owned(),
        model: "gpt".to_owned(),
        extra_information: "debug".to_owned(),
        body: value(body),
        response: None,
    };
    let error = content_policy_error(&input);
    assert_eq!(
        error.message,
        format!("litellm.BadRequestError: litellm.ContentPolicyViolationError: {rendered}")
    );
}

#[rstest]
#[case("a'b", "\"a'b\"")]
#[case("a'b\"c", "'a\\'b\"c'")]
fn azure_policy_nested_string_uses_python_repr(#[case] value: &str, #[case] expected: &str) {
    assert_eq!(super::python_string_repr(value), expected);
}

#[test]
fn azure_policy_null_message_uses_python_repr() {
    assert_eq!(super::display_message(&JsonValue::Null), "None");
}

#[rstest]
#[case::strings(
    r#"{"type":"provider-type","code":"provider-code"}"#,
    Some("provider-type"),
    Some("provider-code")
)]
#[case::non_strings(r#"{"type":true,"code":7}"#, None, None)]
#[case::mixed(
    r#"{"type":[],"code":{"reason":"filtered"}}"#,
    Some("invalid_request_error"),
    None
)]
#[case::fallbacks(
    r#"{"type":"","code":null}"#,
    Some("invalid_request_error"),
    Some("content_policy_violation")
)]
fn azure_policy_classification_keeps_provider_wire_fields(
    #[case] body: &str,
    #[case] expected_type: Option<&str>,
    #[case] expected_code: Option<&str>,
) {
    let input = ContentPolicyInput {
        message: "caller".to_owned(),
        model: "gpt".to_owned(),
        extra_information: "debug".to_owned(),
        body: value(body),
        response: None,
    };
    let error = content_policy_error(&input);
    let ErrorClass::ContentPolicyViolation(context) = error.classification else {
        panic!("expected content-policy classification");
    };
    assert_eq!(context.body_type.as_deref(), expected_type);
    assert_eq!(context.body_code.as_deref(), expected_code);
}
