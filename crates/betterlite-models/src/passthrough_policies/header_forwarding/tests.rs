#![cfg(test)]

use indexmap::IndexMap;

use super::forward_headers_from_request;

fn headers(fields: &[(&str, &str)]) -> IndexMap<String, String> {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn forwarding_disabled_keeps_only_the_custom_headers() {
    let request = headers(&[
        ("content-length", "12"),
        ("host", "client.example"),
        ("accept-encoding", "br"),
        ("x-request-id", "abc"),
    ]);
    let custom = headers(&[("x-route", "one")]);
    assert_eq!(
        forward_headers_from_request(&request, custom, false),
        headers(&[("x-route", "one")])
    );
}

#[test]
fn forwarding_enabled_drops_the_three_protocol_headers() {
    let request = headers(&[
        ("content-length", "12"),
        ("host", "client.example"),
        ("accept-encoding", "br"),
        ("x-request-id", "abc"),
    ]);
    assert_eq!(
        forward_headers_from_request(&request, IndexMap::new(), true),
        headers(&[("x-request-id", "abc")])
    );
}

#[test]
fn a_custom_header_removes_the_request_header_in_any_case() {
    let request = headers(&[
        ("X-Custom", "from-request"),
        ("x-other", "kept"),
        ("content-type", "application/json"),
    ]);
    let custom = headers(&[("x-custom", "from-route"), ("CONTENT-TYPE", "text/plain")]);
    assert_eq!(
        forward_headers_from_request(&request, custom, true),
        headers(&[
            ("x-other", "kept"),
            ("x-custom", "from-route"),
            ("CONTENT-TYPE", "text/plain")
        ])
    );
}

#[test]
fn request_headers_come_before_custom_headers() {
    let request = headers(&[("x-first", "1"), ("x-second", "2")]);
    let custom = headers(&[("x-third", "3")]);
    let merged = forward_headers_from_request(&request, custom, true);
    assert_eq!(
        merged.keys().map(String::as_str).collect::<Vec<_>>(),
        ["x-first", "x-second", "x-third"]
    );
}

#[test]
fn passthrough_headers_are_stripped_even_when_forwarding_is_disabled() {
    let request = headers(&[("x-request-id", "abc"), ("x-pass-anthropic-beta", "beta")]);
    let custom = headers(&[("x-route", "one")]);
    assert_eq!(
        forward_headers_from_request(&request, custom, false),
        headers(&[("x-route", "one"), ("anthropic-beta", "beta")])
    );
}

#[test]
fn passthrough_headers_override_the_forwarded_ones() {
    let request = headers(&[
        ("anthropic-beta", "general"),
        ("x-pass-anthropic-beta", "explicit"),
    ]);
    let merged = forward_headers_from_request(&request, IndexMap::new(), true);
    assert_eq!(
        merged.get("anthropic-beta").map(String::as_str),
        Some("explicit")
    );
}

#[test]
fn a_prefixed_header_is_also_forwarded_verbatim_when_forwarding_is_enabled() {
    let request = headers(&[("x-pass-anthropic-beta", "beta")]);
    let merged = forward_headers_from_request(&request, IndexMap::new(), true);
    assert_eq!(
        merged,
        headers(&[
            ("x-pass-anthropic-beta", "beta"),
            ("anthropic-beta", "beta")
        ])
    );
}

#[test]
fn protected_passthrough_headers_never_reach_the_target() {
    let request = headers(&[
        ("x-pass-authorization", "Bearer mine"),
        ("x-pass-x-amz-security-token", "token"),
        ("x-pass-host", "elsewhere.example"),
        ("x-pass-kept", "yes"),
    ]);
    let merged = forward_headers_from_request(&request, IndexMap::new(), false);
    assert_eq!(merged, headers(&[("kept", "yes")]));
}

#[test]
fn a_custom_header_shadowing_a_prefixed_one_suppresses_it_after_forwarding() {
    let request = headers(&[("x-pass-foo", "from-request")]);
    let custom = headers(&[("x-pass-foo", "from-route")]);
    let merged = forward_headers_from_request(&request, custom, true);
    assert_eq!(merged, headers(&[("x-pass-foo", "from-route")]));
}

#[test]
fn an_empty_target_name_is_kept() {
    let request = headers(&[("x-pass-", "value")]);
    let merged = forward_headers_from_request(&request, IndexMap::new(), false);
    assert_eq!(merged, headers(&[("", "value")]));
}
