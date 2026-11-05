#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::prompt_security_files::log::redact_headers;

fn headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
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
    let redacted = redact_headers(&headers(&[(name, "secret-app-id")]));
    assert_eq!(redacted, headers(&[(name, "REDACTED")]));
}

#[test]
fn redact_headers_keeps_every_other_header_as_sent() {
    let sent = headers(&[
        ("APP-ID", "secret-app-id"),
        ("X-LiteLLM-Key-Alias", "alias"),
        ("Content-Type", "application/json"),
    ]);
    let redacted = redact_headers(&sent);
    assert_eq!(redacted["X-LiteLLM-Key-Alias"], "alias");
    assert_eq!(redacted["Content-Type"], "application/json");
    assert_eq!(redacted["APP-ID"], "REDACTED");
}

#[test]
fn redact_headers_leaves_the_caller_map_untouched() {
    let sent = headers(&[("APP-ID", "secret-app-id")]);
    let redacted = redact_headers(&sent);
    assert_ne!(redacted, sent);
    assert_eq!(sent["APP-ID"], "secret-app-id");
}
