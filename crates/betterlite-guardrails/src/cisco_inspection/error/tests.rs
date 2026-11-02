#![cfg(test)]

use crate::cisco_inspection::error::ApiError;

#[test]
fn api_error_displays_its_message() {
    let error = ApiError("Cisco AI Defense chat API request failed: boom".to_owned());
    assert_eq!(
        error.to_string(),
        "Cisco AI Defense chat API request failed: boom"
    );
}

#[test]
fn api_error_equality_compares_messages() {
    assert_eq!(ApiError("same".to_owned()), ApiError("same".to_owned()));
    assert_ne!(ApiError("a".to_owned()), ApiError("b".to_owned()));
}
