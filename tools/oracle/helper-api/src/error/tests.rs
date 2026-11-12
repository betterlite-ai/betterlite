#![cfg(test)]

use serde_json::Value;

use super::OrHelperError as _;

type Outcome = Result<(), &'static str>;

#[test]
fn or_value_error_writes_type_param_and_code_as_null() {
    let error = Outcome::Err("bad value")
        .or_value_error()
        .expect_err("the result should be an error");
    let serialized = serde_json::to_value(error).expect("the helper error should serialize");
    let expected: Value = serde_json::from_str(
        r#"{"class":"ValueError","type":null,"param":null,"code":null,"message":"bad value"}"#,
    )
    .expect("the expected JSON should parse");
    assert_eq!(serialized, expected);
}

#[test]
fn or_config_error_leaves_type_param_and_code_out() {
    let error = Outcome::Err("bad config")
        .or_config_error()
        .expect_err("the result should be an error");
    let serialized = serde_json::to_value(error).expect("the helper error should serialize");
    let expected: Value = serde_json::from_str(r#"{"class":"ConfigError","message":"bad config"}"#)
        .expect("the expected JSON should parse");
    assert_eq!(serialized, expected);
}
