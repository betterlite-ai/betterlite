#![cfg(test)]

use super::{
    IdentityRowsDecodeError, decode_identity_rows, key::LiteLLMVerificationToken,
    user::LiteLLMUserTable,
};
use rstest::rstest;
use rust_decimal::Decimal;
use std::fs;
use std::path::PathBuf;

#[test]
fn identity_rows_defaults_and_password_exclusion() {
    let user: LiteLLMUserTable = serde_json::from_str(
        r#"{"user_id":"u","password":"secret","spend":null,"models":null,"teams":null}"#,
    )
    .unwrap();
    assert_eq!(user.spend, Decimal::ZERO);
    assert!(serde_json::to_string(&user).unwrap().contains("user_id"));
    assert!(!serde_json::to_string(&user).unwrap().contains("secret"));
}

#[test]
fn identity_money_nonfinite_and_overflow_rejected() {
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"spend":null}"#).unwrap_err();
    serde_json::from_str::<LiteLLMVerificationToken>(r#"{"spend":1e1000}"#).unwrap_err();
}

#[test]
fn native_identity_row_size_and_depth_limits() {
    let oversized = vec![b' '; 0x0010_0001];
    assert!(matches!(
        decode_identity_rows(&oversized),
        Err(IdentityRowsDecodeError::TooLarge { .. })
    ));
    let nested = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(
        decode_identity_rows(nested.as_bytes()),
        Err(IdentityRowsDecodeError::TooDeep)
    ));
}

#[rstest]
fn replays_the_identity_rows_corpus(#[files("fuzz/corpus/identity_rows/*")] path: PathBuf) {
    drop(decode_identity_rows(&fs::read(path).unwrap()));
}
