#![cfg(test)]

use super::{ContentPolicyInput, JsonValue, content_policy_error, extract_azure_error};
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
