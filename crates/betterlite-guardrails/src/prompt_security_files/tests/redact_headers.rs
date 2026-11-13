#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::shared::{header_map, sanitizer};
use crate::prompt_security_files::log::redact_headers;
use crate::prompt_security_files::transport::request_headers;

fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::upper("APP-ID")]
#[case::lower("app-id")]
#[case::mixed("App-Id")]
fn redact_headers_masks_the_app_id_whatever_its_case(#[case] name: &str) {
    let redacted = redact_headers(&header_map(&[(name, "secret-app-id")]));
    assert_eq!(redacted, map(&[("app-id", "REDACTED")]));
}

#[test]
fn redact_headers_keeps_every_other_header_as_sent() {
    let sent = header_map(&[
        ("APP-ID", "secret-app-id"),
        ("X-LiteLLM-Key-Alias", "alias"),
        ("Content-Type", "application/json"),
    ]);
    let redacted = redact_headers(&sent);
    assert_eq!(redacted["x-litellm-key-alias"], "alias");
    assert_eq!(redacted["content-type"], "application/json");
    assert_eq!(redacted["app-id"], "REDACTED");
}

#[test]
fn redact_headers_leaves_the_caller_map_untouched() {
    let sent = header_map(&[("APP-ID", "secret-app-id")]);
    let redacted = redact_headers(&sent);
    assert_eq!(redacted["app-id"], "REDACTED");
    assert_eq!(sent["app-id"], "secret-app-id");
}

#[test]
fn request_headers_marks_the_api_key_sensitive_and_never_plain() {
    let headers =
        request_headers(&sanitizer("http://unused.test"), Some("alias")).expect("headers");
    assert_eq!(headers["app-id"], "vendor-key");
    assert!(headers["app-id"].is_sensitive());
    assert_eq!(headers["x-litellm-key-alias"], "alias");
    assert!(!headers["x-litellm-key-alias"].is_sensitive());
    let shown = format!("{headers:?}");
    assert!(!shown.contains("vendor-key"));
    assert!(shown.contains("Sensitive"));
}
