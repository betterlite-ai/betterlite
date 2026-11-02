#[cfg(test)]
use rstest::rstest;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::test_object;

#[cfg(test)]
#[rstest]
#[case::present(test_object([("type", json!("function_call"))]), Some(json!("function_call")))]
#[case::missing(json!({}), None)]
#[case::not_an_object(json!("plain text"), None)]
fn output_item_type_reads_the_type_field(#[case] item: Value, #[case] expected: Option<Value>) {
    assert_eq!(output_item_type(&item), expected);
}

#[cfg(test)]
#[rstest]
#[case::present(test_object([("output", Value::Array(vec![json!("item")]))]), json!(["item"]))]
#[case::missing(json!({}), json!([]))]
#[case::not_an_object(json!(42_i64), json!([]))]
fn response_output_reads_the_output_field_with_an_empty_default(
    #[case] response: Value,
    #[case] expected: Value,
) {
    assert_eq!(response_output(&response), expected);
}

#[cfg(test)]
#[rstest]
#[case::string(Some(json!("raw text")), "raw text")]
#[case::none(None, "")]
#[case::null(Some(json!(null)), "")]
#[case::number(Some(json!(5_i64)), "5")]
#[case::object(Some(test_object([("code", json!("print(1)"))])), r#"{"code":"print(1)"}"#)]
fn tool_call_arguments_converts_arguments_to_a_string(
    #[case] arguments: Option<Value>,
    #[case] expected: &str,
) {
    assert_eq!(tool_call_arguments(arguments.as_ref()), expected);
}

#[cfg(test)]
#[test]
fn narrow_tool_call_preserves_id_and_call_id_and_overrides_the_name() {
    let tool_call = test_object([
        ("id", json!("call_1")),
        ("call_id", json!("resp_1")),
        ("name", json!("code_interpreter")),
        ("arguments", json!("{\"code\": \"1\"}")),
    ]);
    let narrowed = narrow_tool_call(tool_call.as_object().unwrap());
    assert_eq!(narrowed.id, Some("call_1".to_owned()));
    assert_eq!(narrowed.call_id, Some("resp_1".to_owned()));
    assert_eq!(narrowed.name, CODE_EXECUTION_TOOL_NAME);
    assert_eq!(narrowed.arguments, "{\"code\": \"1\"}");
}

#[cfg(test)]
#[test]
fn narrow_tool_call_drops_non_string_identifiers() {
    let tool_call = test_object([("id", json!(1_i64)), ("call_id", json!(null))]);
    let narrowed = narrow_tool_call(tool_call.as_object().unwrap());
    assert_eq!(narrowed.id, None);
    assert_eq!(narrowed.call_id, None);
}

#[cfg(test)]
#[rstest]
#[case::metadata(
    test_object([("metadata", test_object([("session_id", json!("s1"))]))]),
    Some("s1".to_owned())
)]
#[case::litellm_metadata(
    test_object([("litellm_metadata", test_object([("session_id", json!("s2"))]))]),
    Some("s2".to_owned())
)]
#[case::metadata_preferred_over_litellm_metadata(
    test_object([
        ("metadata", test_object([("session_id", json!("s1"))])),
        ("litellm_metadata", test_object([("session_id", json!("s2"))])),
    ]),
    Some("s1".to_owned())
)]
#[case::absent(json!({}), None)]
#[case::non_string(
    test_object([("metadata", test_object([("session_id", json!(5_i64))]))]),
    None
)]
#[case::empty_string_is_given_as_is(
    test_object([("metadata", test_object([("session_id", json!(""))]))]),
    Some(String::new())
)]
fn extract_session_id_prefers_metadata_over_litellm_metadata(
    #[case] kwargs: Value,
    #[case] expected: Option<String>,
) {
    assert_eq!(extract_session_id(kwargs.as_object().unwrap()), expected);
}

#[cfg(test)]
#[rstest]
#[case::present(test_object([("user_api_key_hash", json!("hash1"))]), "hash1")]
#[case::absent(json!({}), "")]
#[case::non_string(test_object([("user_api_key_hash", json!(5_i64))]), "")]
fn extract_identity_reads_the_user_api_key_hash(#[case] kwargs: Value, #[case] expected: &str) {
    assert_eq!(extract_identity(kwargs.as_object().unwrap()), expected);
}
