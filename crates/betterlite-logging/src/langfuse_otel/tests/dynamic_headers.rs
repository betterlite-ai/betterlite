#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use super::{BASIC_AUTH, PUBLIC_KEY, SECRET_KEY, params, text};
use crate::langfuse_otel::dynamic::construct_dynamic_otel_headers;
use crate::langfuse_otel::error::LangfuseOtelError;

#[test]
fn dynamic_headers_with_both_keys_return_the_langfuse_header_set() {
    let headers = construct_dynamic_otel_headers(&params(&[
        ("langfuse_public_key", text(PUBLIC_KEY)),
        ("langfuse_secret_key", text(SECRET_KEY)),
        ("langfuse_host", text("ignored.example.com")),
    ]))
    .expect("headers");
    let entries: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.expose_secret()))
        .collect();
    assert_eq!(
        entries,
        vec![
            ("Authorization", BASIC_AUTH),
            ("x-langfuse-ingestion-version", "4"),
        ]
    );
}

#[rstest]
#[case::no_params(Json::Null)]
#[case::empty_params(params(&[]))]
#[case::missing_secret(params(&[("langfuse_public_key", text(PUBLIC_KEY))]))]
#[case::missing_public(params(&[("langfuse_secret_key", text(SECRET_KEY))]))]
#[case::empty_secret(params(&[("langfuse_public_key", text(PUBLIC_KEY)), ("langfuse_secret_key", text(""))]))]
#[case::empty_public(params(&[("langfuse_public_key", text("")), ("langfuse_secret_key", text(SECRET_KEY))]))]
#[case::null_secret(params(&[("langfuse_public_key", text(PUBLIC_KEY)), ("langfuse_secret_key", Json::Null)]))]
fn dynamic_headers_without_both_keys_are_empty(#[case] given: Json) {
    let headers = construct_dynamic_otel_headers(&given).expect("headers");
    assert!(headers.is_empty());
}

#[rstest]
#[case::number_key(
    params(&[("langfuse_public_key", Json::Number(Number::from(1_i32))), ("langfuse_secret_key", text(SECRET_KEY))]),
    "langfuse_public_key"
)]
#[case::list_secret(
    params(&[("langfuse_public_key", text(PUBLIC_KEY)), ("langfuse_secret_key", Json::List(Vec::new()))]),
    "langfuse_secret_key"
)]
fn dynamic_headers_reject_a_credential_of_the_wrong_type(#[case] given: Json, #[case] field: &str) {
    let error = construct_dynamic_otel_headers(&given).expect_err("wrong type");
    assert!(matches!(
        error,
        LangfuseOtelError::DynamicFieldType { field: named } if named == field
    ));
}

#[test]
fn dynamic_headers_reject_params_that_are_not_an_object() {
    let error = construct_dynamic_otel_headers(&text("pk")).expect_err("not an object");
    assert!(matches!(error, LangfuseOtelError::DynamicParamsType));
}
