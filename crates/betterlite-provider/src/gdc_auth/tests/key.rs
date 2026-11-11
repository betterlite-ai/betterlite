use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::gdc_auth::error::GdcAuthError;
use crate::gdc_auth::key::{GdcKey, MAX_KEY_BYTES, decode_gdc_key};

use super::support::{P256_SEC1, descriptor_json, descriptor_with_ca};

fn status(error: &GdcAuthError) -> u16 {
    error.class().metadata().status_code
}

#[test]
fn a_plain_token_is_a_bearer_key_verbatim() {
    let GdcKey::Bearer(token) = decode_gdc_key(b"sk-plain-token", "gdc/model").unwrap() else {
        panic!("expected a bearer key");
    };
    assert_eq!(token.expose_secret(), "sk-plain-token");
}

#[test]
fn an_empty_token_is_a_bearer_key() {
    let GdcKey::Bearer(token) = decode_gdc_key(b"", "gdc/model").unwrap() else {
        panic!("expected a bearer key");
    };
    assert_eq!(token.expose_secret(), "");
}

#[rstest]
#[case::json_string("\"abc\"")]
#[case::json_array("[1,2]")]
#[case::json_null("null")]
#[case::json_number("42")]
#[case::empty_object("{}")]
#[case::other_google_type("{\"type\":\"service_account\",\"format_version\":\"1\"}")]
#[case::missing_fields("{\"type\":\"gdch_service_account\"}")]
#[case::nan("NaN")]
#[case::infinity("Infinity")]
#[case::negative_infinity(" -Infinity ")]
fn rejected_keys_are_authentication_failures(#[case] api_key: &str) {
    let error = decode_gdc_key(api_key.as_bytes(), "gdc/model").unwrap_err();
    assert_eq!(status(&error), 401);
}

#[test]
fn valid_json_nested_past_the_parser_depth_limit_is_not_a_bearer_key() {
    let nested = format!("{}{}", "[".repeat(200), "]".repeat(200));
    let error = decode_gdc_key(nested.as_bytes(), "gdc/model").unwrap_err();
    assert_eq!(status(&error), 401);
}

#[test]
fn an_unsupported_format_version_is_refused() {
    let json = descriptor_json("project-a", P256_SEC1)
        .replace("\"format_version\":\"1\"", "\"format_version\":\"2\"");
    let error = decode_gdc_key(json.as_bytes(), "gdc/model").unwrap_err();
    assert_eq!(status(&error), 401);
}

#[test]
fn a_numeric_format_version_is_refused() {
    let json = descriptor_json("project-a", P256_SEC1)
        .replace("\"format_version\":\"1\"", "\"format_version\":1");
    let error = decode_gdc_key(json.as_bytes(), "gdc/model").unwrap_err();
    assert_eq!(status(&error), 401);
}

#[test]
fn duplicate_credential_fields_are_refused() {
    let json = descriptor_json("project-a", P256_SEC1).replace(
        "\"type\":\"gdch_service_account\"",
        "\"type\":\"gdch_service_account\",\"type\":\"gdch_service_account\"",
    );
    let error = decode_gdc_key(json.as_bytes(), "gdc/model").unwrap_err();
    assert_eq!(status(&error), 401);
}

#[test]
fn a_complete_descriptor_keeps_every_field() {
    let GdcKey::ServiceAccount(descriptor) = decode_gdc_key(
        descriptor_json("project-a", P256_SEC1).as_bytes(),
        "gdc/model",
    )
    .unwrap() else {
        panic!("expected a service account");
    };
    assert_eq!(descriptor.private_key_id, "key-id");
    assert_eq!(descriptor.private_key.expose_secret(), P256_SEC1);
    assert_eq!(descriptor.name, "service-identity");
    assert_eq!(descriptor.project, "project-a");
    assert_eq!(descriptor.token_uri, super::support::TOKEN_URI);
    assert_eq!(descriptor.ca_cert_path, None);
}

#[test]
fn a_descriptor_can_name_the_local_ca_certificate() {
    let GdcKey::ServiceAccount(descriptor) = decode_gdc_key(
        descriptor_with_ca("project-a", P256_SEC1, "ca.pem").as_bytes(),
        "gdc/model",
    )
    .unwrap() else {
        panic!("expected a service account");
    };
    assert_eq!(descriptor.ca_cert_path.as_deref(), Some("ca.pem"));
}

#[test]
fn non_utf8_credential_bytes_are_refused() {
    let error = decode_gdc_key(&[0xFF, 0xFE, 0x00], "gdc/model").unwrap_err();
    assert_eq!(status(&error), 401);
}

#[test]
fn the_size_limit_keeps_the_boundary_and_refuses_one_byte_more() {
    let at_limit = vec![b'a'; MAX_KEY_BYTES];
    let _key = decode_gdc_key(&at_limit, "gdc/model").expect("a key at the limit");
    let over = vec![b'a'; MAX_KEY_BYTES.saturating_add(1)];
    assert_eq!(
        status(&decode_gdc_key(&over, "gdc/model").unwrap_err()),
        401
    );
}

#[test]
fn the_private_key_never_reaches_the_error_text() {
    let json = descriptor_json("project-a", "super-secret-key-material")
        .replace("\"format_version\":\"1\"", "\"format_version\":\"9\"");
    let error = decode_gdc_key(json.as_bytes(), "gdc/model").unwrap_err();
    assert!(!error.to_string().contains("super-secret-key-material"));
}
