#![cfg(test)]

use betterlite_prompts::arguments::json::Json;

use super::RequestError;

#[test]
fn a_validator_error_displays_its_message_without_a_pydantic_prefix() {
    let error = RequestError::Value("args is required for stdio transport".to_owned());
    assert_eq!(error.to_string(), "args is required for stdio transport");
}

#[test]
fn a_shape_error_displays_the_serde_message() {
    let shape = serde_json::from_str::<Json>("{").unwrap_err();
    let expected = shape.to_string();
    assert_eq!(RequestError::Shape(shape).to_string(), expected);
}
