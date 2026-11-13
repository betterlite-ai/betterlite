#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::{Map, Value};

use crate::case::{BodyKind, Compare, Ignore, RequestBody};
use crate::compare::compare;
use crate::http::Response;
use crate::mock::ProviderRequest;
use crate::multipart_oracle::request::request_body;

const TARGETS_DIFFER: &str = "provider request targets differ";
const BODIES_DIFFER: &str = "provider request bodies differ";
const HEADERS_DIFFER: &str = "provider header values differ";
const GENERATED_FILENAMES_DIFFER: &str = "generated filenames differ in shape";
const NO_CAPTURED_SELECTOR: &str =
    "compare.ignore names no captured provider request, part or header";
const REPEATED_SELECTOR: &str = "compare.ignore repeats a selector";

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn response(body: Value) -> Response {
    Response {
        status: 200,
        headers: BTreeMap::new(),
        body,
    }
}

fn request(body: Value) -> ProviderRequest {
    ProviderRequest {
        method: "POST".to_owned(),
        path: "/v1/chat/completions".to_owned(),
        headers: BTreeMap::new(),
        body,
    }
}

fn with_header(mut request: ProviderRequest, name: &str, value: &str) -> ProviderRequest {
    request.headers.insert(name.to_owned(), value.to_owned());
    request
}

fn authorized(body: Value, sent: Option<&str>) -> ProviderRequest {
    let request = request(body);
    match sent {
        Some(header) => with_header(request, "authorization", header),
        None => request,
    }
}

fn comparing(selectors: &[&str], upstream_headers: &[&str]) -> Compare {
    Compare {
        headers: Vec::new(),
        headers_present: Vec::new(),
        upstream_headers: upstream_headers
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        ignore: selectors
            .iter()
            .map(|pointer| Ignore {
                pointer: (*pointer).to_owned(),
                why: "the mock generates a different value on each run".to_owned(),
            })
            .collect(),
        contracts: None,
    }
}

fn failure(
    left: &[ProviderRequest],
    right: &[ProviderRequest],
    rules: &Compare,
) -> Option<(String, String)> {
    let empty = response(Value::Null);
    compare(&empty, &empty, left, right, rules)
        .err()
        .map(|difference| (difference.pointer, difference.message))
}

fn part_spec(name: &str, filename: Option<&str>, content_type: Option<&str>, text: &str) -> Value {
    let mut part = Map::new();
    part.insert("name".to_owned(), Value::String(name.to_owned()));
    if let Some(specified) = filename {
        part.insert("filename".to_owned(), Value::String(specified.to_owned()));
    }
    if let Some(declared) = content_type {
        part.insert(
            "content_type".to_owned(),
            Value::String(declared.to_owned()),
        );
    }
    part.insert("text".to_owned(), Value::String(text.to_owned()));
    Value::Object(part)
}

fn field_part(name: &str, text: &str) -> Value {
    part_spec(name, None, None, text)
}

fn file_part(filename: Option<&str>, text: &str) -> Value {
    part_spec("file", filename, Some("text/plain"), text)
}

fn upload(parts: Value) -> ProviderRequest {
    let mut document = Map::new();
    document.insert("parts".to_owned(), parts);
    let RequestBody::Raw {
        content_type,
        bytes,
    } = request_body(&Value::Object(document), BodyKind::Multipart).unwrap()
    else {
        panic!("a multipart specification encodes to raw bytes");
    };
    ProviderRequest {
        method: "POST".to_owned(),
        path: "/v1/files".to_owned(),
        headers: BTreeMap::from([("content-type".to_owned(), content_type)]),
        body: Value::Array(bytes.iter().map(|byte| Value::from(*byte)).collect()),
    }
}

#[test]
fn body_selector_skips_one_body() {
    let left = vec![
        request(value(r#"{"prompt": "first"}"#)),
        request(value(r#"{"prompt": "same"}"#)),
    ];
    let mut right = vec![
        request(value(r#"{"prompt": "second"}"#)),
        request(value(r#"{"prompt": "same"}"#)),
    ];
    let rules = comparing(&["/upstream/0/body"], &[]);
    assert_eq!(failure(&left, &right, &rules), None);
    right[1] = request(value(r#"{"prompt": "changed"}"#));
    assert_eq!(
        failure(&left, &right, &rules),
        Some(("/upstream/1/body".to_owned(), BODIES_DIFFER.to_owned()))
    );
}

#[rstest]
#[case::changed_path("/v1/other", None, "/upstream/0", TARGETS_DIFFER)]
#[case::changed_authorization(
    "/v1/chat/completions",
    Some("second-token"),
    "/upstream/0/headers/authorization",
    HEADERS_DIFFER
)]
fn body_selector_keeps_target_and_headers(
    #[case] right_path: &str,
    #[case] right_token: Option<&str>,
    #[case] pointer: &str,
    #[case] message: &str,
) {
    let body = value(r#"{"prompt": "first"}"#);
    let left = vec![authorized(body, Some("first-token"))];
    let mut right_request = authorized(
        value(r#"{"prompt": "second"}"#),
        Some(right_token.unwrap_or("first-token")),
    );
    right_request.path = right_path.to_owned();
    let rules = comparing(&["/upstream/0/body"], &["authorization"]);
    assert_eq!(
        failure(&left, &[right_request], &rules),
        Some((pointer.to_owned(), message.to_owned()))
    );
}

#[rstest]
#[case::hex_text(
    "0123456789ab4def8123456789abcdef.txt",
    "aaaaaaaaaaaa4aaaaaaaaaaaaaaaaaaa.txt"
)]
#[case::hyphenated_png(
    "12345678-1234-4123-8123-123456789abc.png",
    "abcdef01-2345-4678-9abc-def012345678.png"
)]
fn filename_selector_accepts_generated_names(#[case] left_name: &str, #[case] right_name: &str) {
    let (left, right) = uploads(Some(left_name), Some(right_name));
    let rules = comparing(&["/upstream/0/parts/1/filename"], &[]);
    assert_eq!(failure(&left, &right, &rules), None);
}

fn uploads(
    left_name: Option<&str>,
    right_name: Option<&str>,
) -> (Vec<ProviderRequest>, Vec<ProviderRequest>) {
    let left = vec![upload(Value::Array(vec![
        field_part("field", "value"),
        file_part(left_name, "payload"),
    ]))];
    let right = vec![upload(Value::Array(vec![
        field_part("field", "value"),
        file_part(right_name, "payload"),
    ]))];
    (left, right)
}

#[rstest]
#[case::different_extensions(
    Some("0123456789ab4def8123456789abcdef.txt"),
    Some("aaaaaaaaaaaa4aaaaaaaaaaaaaaaaaaa.png"),
    GENERATED_FILENAMES_DIFFER
)]
#[case::not_a_uuid(
    Some("notes.txt"),
    Some("0123456789ab4def8123456789abcdef.txt"),
    GENERATED_FILENAMES_DIFFER
)]
#[case::version_one(
    Some("12345678-1234-1123-8123-123456789abc.txt"),
    Some("abcdef01-2345-4678-9abc-def012345678.txt"),
    GENERATED_FILENAMES_DIFFER
)]
#[case::no_extension(
    Some("0123456789ab4def8123456789abcdef"),
    Some("aaaaaaaaaaaa4aaaaaaaaaaaaaaaaaaa"),
    GENERATED_FILENAMES_DIFFER
)]
#[case::hex_against_hyphenated(
    Some("0123456789ab4def8123456789abcdef.txt"),
    Some("01234567-89ab-4def-8123-456789abcdef.txt"),
    GENERATED_FILENAMES_DIFFER
)]
#[case::missing_filename(
    Some("0123456789ab4def8123456789abcdef.txt"),
    None,
    NO_CAPTURED_SELECTOR
)]
fn filename_selector_refuses_other_names(
    #[case] left_name: Option<&str>,
    #[case] right_name: Option<&str>,
    #[case] message: &str,
) {
    let (left, right) = uploads(left_name, right_name);
    let rules = comparing(&["/upstream/0/parts/1/filename"], &[]);
    assert_eq!(
        failure(&left, &right, &rules),
        Some((
            "/upstream/0/parts/1/filename".to_owned(),
            message.to_owned()
        ))
    );
}

#[rstest]
#[case::name("renamed", "text/plain", "payload", "/upstream/0/parts/1/name")]
#[case::content_type(
    "file",
    "application/octet-stream",
    "payload",
    "/upstream/0/parts/1/content_type"
)]
#[case::body("file", "text/plain", "changed", "/upstream/0/parts/1/body")]
fn filename_selector_keeps_part_fields(
    #[case] name: &str,
    #[case] content_type: &str,
    #[case] text: &str,
    #[case] pointer: &str,
) {
    let left = vec![upload(Value::Array(vec![
        field_part("field", "value"),
        file_part(Some("12345678-1234-4123-8123-123456789abc.png"), "payload"),
    ]))];
    let right = vec![upload(Value::Array(vec![
        field_part("field", "value"),
        part_spec(
            name,
            Some("abcdef01-2345-4678-9abc-def012345678.png"),
            Some(content_type),
            text,
        ),
    ]))];
    let rules = comparing(&["/upstream/0/parts/1/filename"], &[]);
    assert_eq!(
        failure(&left, &right, &rules),
        Some((pointer.to_owned(), BODIES_DIFFER.to_owned()))
    );
}

#[rstest]
#[case::listed_authorization("authorization", true)]
#[case::unlisted_channel_token("x-mcp-channel-token", false)]
fn header_selector_checks_presence(#[case] name: &str, #[case] listed: bool) {
    let selector = format!("/upstream/0/headers/{name}");
    let upstream_headers: Vec<&str> = if listed { vec![name] } else { Vec::new() };
    let left = vec![with_header(
        request(value(r#"{"prompt": "same"}"#)),
        name,
        "first-token",
    )];
    let right = vec![with_header(
        request(value(r#"{"prompt": "same"}"#)),
        name,
        "second-token",
    )];
    let rules = comparing(&[selector.as_str()], &upstream_headers);
    assert_eq!(failure(&left, &right, &rules), None);
    assert_other_requests_keep_comparing_the_header();
    assert_selected_request_keeps_body_and_target_compared();
}

fn assert_other_requests_keep_comparing_the_header() {
    let left = vec![
        authorized(value(r#"{"prompt": "same"}"#), Some("first")),
        authorized(value(r#"{"prompt": "same"}"#), Some("first")),
    ];
    let right = vec![
        authorized(value(r#"{"prompt": "same"}"#), Some("second")),
        authorized(value(r#"{"prompt": "same"}"#), Some("changed")),
    ];
    let rules = comparing(&["/upstream/0/headers/authorization"], &["authorization"]);
    assert_eq!(
        failure(&left, &right, &rules),
        Some((
            "/upstream/1/headers/authorization".to_owned(),
            HEADERS_DIFFER.to_owned()
        ))
    );
}

fn assert_selected_request_keeps_body_and_target_compared() {
    let left = vec![authorized(value(r#"{"prompt": "first"}"#), Some("first"))];
    let rules = comparing(&["/upstream/0/headers/authorization"], &["authorization"]);
    let changed_body = vec![authorized(value(r#"{"prompt": "second"}"#), Some("second"))];
    assert_eq!(
        failure(&left, &changed_body, &rules),
        Some(("/upstream/0/body".to_owned(), BODIES_DIFFER.to_owned()))
    );
    let mut changed_path = authorized(value(r#"{"prompt": "first"}"#), Some("second"));
    changed_path.path = "/v1/other".to_owned();
    assert_eq!(
        failure(&left, &[changed_path], &rules),
        Some(("/upstream/0".to_owned(), TARGETS_DIFFER.to_owned()))
    );
}

#[rstest]
#[case::left_only(
    Some("first"),
    None,
    "/upstream/0/headers/authorization",
    NO_CAPTURED_SELECTOR
)]
#[case::right_only(
    None,
    Some("second"),
    "/upstream/0/headers/authorization",
    NO_CAPTURED_SELECTOR
)]
#[case::neither(None, None, "/upstream/0/headers/authorization", NO_CAPTURED_SELECTOR)]
#[case::uppercase_name(
    Some("first"),
    Some("second"),
    "/upstream/0/headers/Authorization",
    NO_CAPTURED_SELECTOR
)]
fn header_selector_refuses_missing_header(
    #[case] left_token: Option<&str>,
    #[case] right_token: Option<&str>,
    #[case] selector: &str,
    #[case] message: &str,
) {
    let left = vec![authorized(value(r#"{"prompt": "same"}"#), left_token)];
    let right = vec![authorized(value(r#"{"prompt": "same"}"#), right_token)];
    assert_eq!(
        failure(&left, &right, &comparing(&[selector], &[])),
        Some((selector.to_owned(), message.to_owned()))
    );
}

#[rstest]
#[case::zero_captures(&["/upstream/0/body"], 0, NO_CAPTURED_SELECTOR)]
#[case::json_capture(&["/upstream/0/parts/0/filename"], 1, NO_CAPTURED_SELECTOR)]
#[case::leading_zero(&["/upstream/00/body"], 1, NO_CAPTURED_SELECTOR)]
#[case::headers_without_name(&["/upstream/0/headers"], 1, NO_CAPTURED_SELECTOR)]
#[case::method(&["/upstream/0/method"], 1, NO_CAPTURED_SELECTOR)]
#[case::repeated(
    &["/upstream/0/body", "/upstream/0/body"],
    1,
    REPEATED_SELECTOR
)]
fn unused_selector_is_refused(
    #[case] selectors: &[&str],
    #[case] requests: usize,
    #[case] message: &str,
) {
    let left = vec![request(value(r#"{"prompt": "same"}"#)); requests];
    let right = left.clone();
    let rules = comparing(selectors, &[]);
    assert_eq!(
        failure(&left, &right, &rules),
        Some((selectors[0].to_owned(), message.to_owned()))
    );
}

#[test]
fn selectors_never_touch_the_response() {
    let left = response(value(r#"{"upstream": [{"body": "first"}]}"#));
    let right = response(value(r#"{"upstream": [{"body": "second"}]}"#));
    let left_requests = vec![request(value(r#"{"prompt": "first"}"#))];
    let right_requests = vec![request(value(r#"{"prompt": "second"}"#))];
    let rules = comparing(&["/upstream/0/body"], &[]);
    let difference = compare(&left, &right, &left_requests, &right_requests, &rules).unwrap_err();
    assert_eq!(difference.pointer, "/upstream/0/body");
    assert_eq!(difference.message, "response bodies differ");
}
