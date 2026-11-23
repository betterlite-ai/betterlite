#![cfg(test)]

use http::HeaderMap;
use http::header::{AUTHORIZATION, CONTENT_TYPE};

use super::{PREVIEW_CHARS, preview, secret_values};

fn headers(pairs: &[(http::HeaderName, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(name.clone(), value.parse().unwrap());
    }
    map
}

#[test]
fn capture_masks_the_credential_header_value() {
    let map = headers(&[
        (AUTHORIZATION, "Bearer abc123"),
        (CONTENT_TYPE, "application/json"),
    ]);
    let secrets = secret_values(&map);
    let body = preview(b"token Bearer abc123 leaked", &secrets);
    assert!(!body.contains("abc123"));
    assert!(body.contains("****"));
}

#[test]
fn capture_masks_the_scheme_stripped_tail_too() {
    let map = headers(&[(AUTHORIZATION, "Bearer abc123")]);
    let secrets = secret_values(&map);
    assert!(secrets.contains(&"Bearer abc123".to_owned()));
    assert!(secrets.contains(&"abc123".to_owned()));
    let body = preview(b"the raw abc123 alone", &secrets);
    assert_eq!(body, "the raw **** alone");
}

#[test]
fn capture_leaves_public_headers_visible() {
    let map = headers(&[(CONTENT_TYPE, "application/json")]);
    assert!(secret_values(&map).is_empty());
    let body = preview(b"content type application/json", &secret_values(&map));
    assert!(body.contains("application/json"));
}

#[test]
fn capture_skips_an_empty_header_value() {
    let map = headers(&[(AUTHORIZATION, "")]);
    assert!(secret_values(&map).is_empty());
}

#[test]
fn capture_reports_an_empty_body() {
    assert_eq!(preview(b"", &[]), "(empty)");
}

#[test]
fn capture_reports_a_non_utf8_body() {
    assert_eq!(
        preview(&[0xff_u8, 0xfe], &[]),
        "(omitted: body is not UTF-8)"
    );
}

#[test]
fn capture_truncates_a_long_body() {
    let body = "x".repeat(PREVIEW_CHARS + 10);
    let reported = preview(body.as_bytes(), &[]);
    assert!(reported.ends_with("...(truncated)"));
    assert_eq!(
        reported.chars().count(),
        PREVIEW_CHARS + "...(truncated)".chars().count()
    );
}

#[test]
fn capture_keeps_a_body_at_the_limit_whole() {
    let body = "y".repeat(PREVIEW_CHARS);
    assert_eq!(preview(body.as_bytes(), &[]), body);
}
