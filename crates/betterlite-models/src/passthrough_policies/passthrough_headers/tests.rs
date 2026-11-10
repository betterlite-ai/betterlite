#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;

use super::{forward_passthrough_headers, passthrough_header_name};

fn headers(fields: &[(&str, &str)]) -> IndexMap<String, String> {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::lower_case("x-pass-anthropic-beta", Some("anthropic-beta"))]
#[case::mixed_case("X-Pass-Anthropic-Beta", Some("anthropic-beta"))]
#[case::upper_case("X-PASS-ANTHROPIC-BETA", Some("anthropic-beta"))]
#[case::prefix_only("x-pass-", Some(""))]
#[case::without_prefix("anthropic-beta", None)]
#[case::prefix_without_dash("x-pass", None)]
#[case::dash_outside_prefix("xpass-anthropic-beta", None)]
#[case::prefix_inside_name("pass-anthropic-beta", None)]
fn the_prefix_is_stripped_and_lowercased(#[case] name: &str, #[case] expected: Option<&str>) {
    assert_eq!(passthrough_header_name(name), expected.map(str::to_owned));
}

#[test]
fn prefixed_headers_are_forwarded_with_the_prefix_removed() {
    let request = headers(&[
        ("x-pass-anthropic-beta", "beta"),
        ("X-Pass-Anthropic-Version", "2023-06-01"),
        ("content-type", "application/json"),
    ]);
    let mut forwarded = IndexMap::new();
    forward_passthrough_headers(&request, &mut forwarded);
    assert_eq!(
        forwarded,
        headers(&[
            ("anthropic-beta", "beta"),
            ("anthropic-version", "2023-06-01")
        ])
    );
}

#[test]
fn protected_targets_are_skipped() {
    let request = headers(&[
        ("x-pass-authorization", "Bearer mine"),
        ("X-Pass-X-Api-Key", "mine"),
        ("x-pass-x-amz-date", "20260101T000000Z"),
        ("x-pass-content-length", "12"),
        ("x-pass-accept-encoding", "br"),
        ("x-pass-ok", "yes"),
    ]);
    let mut forwarded = IndexMap::new();
    forward_passthrough_headers(&request, &mut forwarded);
    assert_eq!(forwarded, headers(&[("ok", "yes")]));
}

#[test]
fn an_existing_header_is_overridden_in_place() {
    let request = headers(&[("x-pass-anthropic-beta", "from-request")]);
    let mut forwarded = headers(&[("anthropic-beta", "from-route")]);
    forward_passthrough_headers(&request, &mut forwarded);
    assert_eq!(forwarded, headers(&[("anthropic-beta", "from-request")]));
}

#[test]
fn a_differently_cased_target_is_a_second_header() {
    let request = headers(&[("x-pass-anthropic-beta", "from-request")]);
    let mut forwarded = headers(&[("Anthropic-Beta", "from-route")]);
    forward_passthrough_headers(&request, &mut forwarded);
    assert_eq!(
        forwarded,
        headers(&[
            ("Anthropic-Beta", "from-route"),
            ("anthropic-beta", "from-request"),
        ])
    );
}

#[test]
fn the_last_prefixed_header_wins_for_one_target() {
    let request = headers(&[("x-pass-x-custom", "first"), ("X-Pass-X-Custom", "second")]);
    let mut forwarded = IndexMap::new();
    forward_passthrough_headers(&request, &mut forwarded);
    assert_eq!(forwarded, headers(&[("x-custom", "second")]));
}
