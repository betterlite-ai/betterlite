#![cfg(test)]

use betterlite_config::sections::request_headers::{Settings, UserHeaderMapping, UserHeaderRole};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use http::{HeaderMap, HeaderName, HeaderValue};
use rstest::rstest;

use super::{IdSettings, coerce_user_id, customer_header_names, request_end_user_id};

fn body(encoded: &str) -> JsonObject {
    serde_json::from_str(encoded).expect("a request body")
}

fn mappings(entries: &[(&str, UserHeaderRole)]) -> Vec<UserHeaderMapping> {
    entries
        .iter()
        .map(|(header_name, role)| UserHeaderMapping {
            header_name: (*header_name).to_owned(),
            litellm_user_role: *role,
        })
        .collect()
}

fn header_settings(entries: &[(&str, UserHeaderRole)], legacy: Option<&str>) -> Settings {
    Settings {
        user_header_mappings: mappings(entries),
        user_header_name: legacy.map(str::to_owned),
        add_user_information: false,
        forward_openai_org_id: false,
        model_group_forwarding: None,
        forward_client_headers: false,
        forward_traceparent: false,
    }
}

fn request_headers(pairs: &[(&str, &str)]) -> HeaderMap {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                HeaderName::from_bytes(name.as_bytes()).expect("a header name"),
                HeaderValue::from_bytes(value.as_bytes()).expect("a header value"),
            )
        })
        .collect()
}

fn resolved(
    encoded: &str,
    headers: &[(&str, &str)],
    settings: &Settings,
    validate: bool,
) -> Option<String> {
    let request_headers = request_headers(headers);
    request_end_user_id(
        &body(encoded),
        Some(&request_headers),
        &IdSettings {
            request_headers: settings,
            validate_end_user_id_in_db: validate,
        },
    )
}

#[test]
fn standard_headers_first() {
    let settings = header_settings(&[("x-mapped", UserHeaderRole::Customer)], Some("x-legacy"));
    let both = [
        ("x-mapped", "mapped"),
        ("x-litellm-end-user-id", "end-user"),
        ("x-litellm-customer-id", "customer"),
    ];
    assert_eq!(
        resolved("{}", &both, &settings, false).as_deref(),
        Some("customer")
    );
    let only_end_user = [
        ("x-mapped", "mapped"),
        ("x-litellm-end-user-id", "end-user"),
    ];
    assert_eq!(
        resolved("{}", &only_end_user, &settings, false).as_deref(),
        Some("end-user")
    );
}

#[test]
fn standard_header_casefold() {
    let settings = header_settings(&[], None);
    let customer = [("X-LiteLLM-Customer-ID", "customer")];
    assert_eq!(
        resolved("{}", &customer, &settings, false).as_deref(),
        Some("customer")
    );
    let end_user = [("X-LiteLLM-END-USER-ID", "end-user")];
    assert_eq!(
        resolved("{}", &end_user, &settings, false).as_deref(),
        Some("end-user")
    );
}

#[test]
fn blank_standard_header_falls_through() {
    let settings = header_settings(&[], None);
    let next_standard = [
        ("x-litellm-customer-id", "   "),
        ("x-litellm-end-user-id", "end-user"),
    ];
    assert_eq!(
        resolved("{}", &next_standard, &settings, false).as_deref(),
        Some("end-user")
    );
    let body_fallback = [("x-litellm-customer-id", " ")];
    assert_eq!(
        resolved(r#"{"user": "body"}"#, &body_fallback, &settings, false).as_deref(),
        Some("body")
    );
}

#[test]
fn customer_mappings_in_order() {
    let settings = header_settings(
        &[
            ("X-First", UserHeaderRole::Customer),
            ("x-second", UserHeaderRole::Customer),
        ],
        None,
    );
    let both = [("x-first", "first"), ("x-second", "second")];
    assert_eq!(
        resolved("{}", &both, &settings, false).as_deref(),
        Some("first")
    );
    let second_only = [("x-second", "second")];
    assert_eq!(
        resolved("{}", &second_only, &settings, false).as_deref(),
        Some("second")
    );
}

#[test]
fn absent_mapped_header_blocks_legacy() {
    let settings = header_settings(&[("x-mapped", UserHeaderRole::Customer)], Some("x-legacy"));
    let headers = [("x-legacy", "legacy")];
    assert_eq!(resolved("{}", &headers, &settings, false), None);
}

#[test]
fn legacy_name_needs_text() {
    let headers = [("x-legacy", "legacy")];
    let exact = header_settings(&[], Some("x-legacy"));
    assert_eq!(
        resolved("{}", &headers, &exact, false).as_deref(),
        Some("legacy")
    );
    let padded = header_settings(&[], Some(" x-legacy "));
    assert_eq!(resolved("{}", &headers, &padded, false), None);
    let blank = header_settings(&[], Some("   "));
    assert_eq!(resolved("{}", &headers, &blank, false), None);
}

#[test]
fn internal_user_mapping_ignored() {
    let settings = header_settings(
        &[("x-internal", UserHeaderRole::InternalUser)],
        Some("x-legacy"),
    );
    let headers = [("x-internal", "internal"), ("x-legacy", "legacy")];
    assert_eq!(
        resolved("{}", &headers, &settings, false).as_deref(),
        Some("legacy")
    );
    let without_legacy = header_settings(&[("x-internal", UserHeaderRole::InternalUser)], None);
    assert_eq!(resolved("{}", &headers, &without_legacy, false), None);
}

#[test]
fn no_headers_skips_header_steps() {
    let settings = header_settings(&[("x-mapped", UserHeaderRole::Customer)], Some("x-legacy"));
    let id_settings = IdSettings {
        request_headers: &settings,
        validate_end_user_id_in_db: false,
    };
    let end_user_id = request_end_user_id(&body(r#"{"user": "body"}"#), None, &id_settings);
    assert_eq!(end_user_id.as_deref(), Some("body"));
    assert_eq!(request_end_user_id(&body("{}"), None, &id_settings), None);
}

#[rstest]
#[case::user(
    r#"{"user": "top", "litellm_metadata": {"user": "lm"}, "metadata": {"user_id": "md"}, "safety_identifier": "sid"}"#,
    Some("top")
)]
#[case::litellm_metadata(
    r#"{"litellm_metadata": {"user": "lm"}, "metadata": {"user_id": "md"}, "safety_identifier": "sid"}"#,
    Some("lm")
)]
#[case::metadata(
    r#"{"metadata": {"user_id": "md"}, "safety_identifier": "sid"}"#,
    Some("md")
)]
#[case::safety_identifier(r#"{"safety_identifier": "sid"}"#, Some("sid"))]
#[case::nothing("{}", None)]
fn body_source_order(#[case] encoded: &str, #[case] expected: Option<&str>) {
    let settings = header_settings(&[], None);
    assert_eq!(
        resolved(encoded, &[], &settings, false).as_deref(),
        expected
    );
}

#[rstest]
#[case::litellm_metadata_text(r#"{"litellm_metadata": "{\"user\": \"parsed\"}"}"#, Some("parsed"))]
#[case::metadata_text(r#"{"metadata": "{\"user_id\": \"parsed\"}"}"#, Some("parsed"))]
#[case::text_list(r#"{"metadata": "[\"md\"]", "safety_identifier": "sid"}"#, Some("sid"))]
#[case::text_scalar(r#"{"metadata": "\"md\"", "safety_identifier": "sid"}"#, Some("sid"))]
#[case::malformed_text(
    r#"{"litellm_metadata": "{\"user\": ", "safety_identifier": "sid"}"#,
    Some("sid")
)]
#[case::number_bucket(r#"{"metadata": 7, "safety_identifier": "sid"}"#, Some("sid"))]
#[case::list_bucket(r#"{"litellm_metadata": [], "safety_identifier": "sid"}"#, Some("sid"))]
fn metadata_string_object_only(#[case] encoded: &str, #[case] expected: Option<&str>) {
    let settings = header_settings(&[], None);
    assert_eq!(
        resolved(encoded, &[], &settings, false).as_deref(),
        expected
    );
}

#[rstest]
#[case::integer(r#"{"user": 7}"#, "7")]
#[case::float(r#"{"user": 1.5}"#, "1.5")]
#[case::trailing_zero(r#"{"user": 1.50}"#, "1.5")]
#[case::exponent(r#"{"user": 1e3}"#, "1000.0")]
#[case::negative_zero(r#"{"user": -0.0}"#, "-0.0")]
#[case::big_integer(
    r#"{"user": 123456789012345678901234567890}"#,
    "123456789012345678901234567890"
)]
#[case::overflowing_float(r#"{"user": 1e999}"#, "Infinity")]
#[case::metadata_number(r#"{"metadata": {"user_id": 1.0}}"#, "1.0")]
fn numbers_use_python_text(#[case] encoded: &str, #[case] expected: &str) {
    let settings = header_settings(&[], None);
    assert_eq!(
        resolved(encoded, &[], &settings, false).as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::padded_text(r#"{"user": " x "}"#, Some("x"))]
#[case::spaces(r#"{"user": "   "}"#, None)]
#[case::empty(r#"{"user": ""}"#, None)]
#[case::boolean(r#"{"user": true}"#, None)]
#[case::list(r#"{"user": ["a"]}"#, None)]
#[case::object(r#"{"user": {"a": 1}}"#, None)]
#[case::null(r#"{"user": null}"#, None)]
#[case::separators(r#"{"user": "\u001c id \u001f"}"#, Some("id"))]
#[case::separators_only(r#"{"user": "\u001c\u001f"}"#, None)]
fn trim_blank_bool_and_structures(#[case] encoded: &str, #[case] expected: Option<&str>) {
    let settings = header_settings(&[], None);
    assert_eq!(
        resolved(encoded, &[], &settings, false).as_deref(),
        expected
    );
}

#[rstest]
#[case::object("{\"a\": 1}", true, None)]
#[case::list("[1]", true, None)]
#[case::padded_object(" {\"a\": 1} ", true, None)]
#[case::object_unvalidated("{\"a\": 1}", false, Some("{\"a\": 1}"))]
#[case::list_unvalidated("[1]", false, Some("[1]"))]
fn json_text_dropped_when_validating(
    #[case] id: &str,
    #[case] validate: bool,
    #[case] expected: Option<&str>,
) {
    let value = Json::Text(id.to_owned());
    assert_eq!(coerce_user_id(&value, validate).as_deref(), expected);
}

#[rstest]
#[case::unterminated("[x")]
#[case::truncated_object("{\"a\":")]
#[case::lone_brace("{")]
fn malformed_json_text_kept(#[case] id: &str) {
    let value = Json::Text(id.to_owned());
    assert_eq!(coerce_user_id(&value, true).as_deref(), Some(id));
}

#[test]
fn header_bytes_decode_as_latin1() {
    let settings = header_settings(&[], None);
    let mut headers = HeaderMap::new();
    headers.append(
        HeaderName::from_static("x-litellm-customer-id"),
        HeaderValue::from_bytes(&[b'c', b'a', b'f', 0xe9]).expect("a latin-1 header value"),
    );
    let end_user_id = request_end_user_id(
        &body("{}"),
        Some(&headers),
        &IdSettings {
            request_headers: &settings,
            validate_end_user_id_in_db: false,
        },
    );
    assert_eq!(end_user_id.as_deref(), Some("café"));
}

#[test]
fn customer_header_names_lowercase_and_skip_empty() {
    let entries = mappings(&[
        ("X-Team", UserHeaderRole::Customer),
        ("", UserHeaderRole::Customer),
        ("x-internal", UserHeaderRole::InternalUser),
    ]);
    assert_eq!(customer_header_names(&entries), vec!["x-team".to_owned()]);
}
