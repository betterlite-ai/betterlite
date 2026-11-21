use betterlite_db::mcp_stored_contracts::JsonValue;
use rstest::rstest;

use super::client_supplied_redirect_uris;

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn uris(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::two_strings(
    r#"["https://client.example/cb","http://127.0.0.1:8080/cb"]"#,
    Some(r#"["https://client.example/cb","http://127.0.0.1:8080/cb"]"#)
)]
#[case::single_string(
    r#"["https://client.example/cb"]"#,
    Some(r#"["https://client.example/cb"]"#)
)]
#[case::empty_array("[]", None)]
#[case::empty_string_element(r#"["https://client.example/cb",""]"#, None)]
#[case::number_element(r#"["https://client.example/cb",5]"#, None)]
#[case::null_element(r#"["https://client.example/cb",null]"#, None)]
#[case::object_element(r#"["https://client.example/cb",{}]"#, None)]
#[case::array_element(r#"["https://client.example/cb",[]]"#, None)]
#[case::string_value(r#""https://client.example/cb""#, None)]
#[case::object_value(r#"{"redirect_uris":[]}"#, None)]
#[case::number_value("5", None)]
#[case::bool_value("true", None)]
#[case::null_value("null", None)]
fn client_supplied_redirect_uris_matches_litellm(
    #[case] input: &str,
    #[case] expected: Option<&str>,
) {
    let value = json(input);
    assert_eq!(
        client_supplied_redirect_uris(Some(&value)),
        expected.map(uris)
    );
}

#[test]
fn client_supplied_redirect_uris_rejects_an_absent_value() {
    assert!(client_supplied_redirect_uris(None).is_none());
}
