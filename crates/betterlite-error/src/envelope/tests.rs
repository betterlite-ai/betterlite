#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{WireField, allowed_response_headers, filter_unsafe_headers, project_provider_headers};

fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

enum Operation {
    Filter,
    Project,
    AllowedFromResponse,
    AllowedFromExplicit,
}

#[rstest]
#[case::drops_raw_set_cookie(Operation::Filter, &[("set-cookie", "a=b")], &[])]
#[case::drops_raw_cors_origin(Operation::Filter, &[("access-control-allow-origin", "*")], &[])]
#[case::drops_prefixed_cookie(Operation::Filter, &[("llm_provider-set-cookie", "a=b")], &[])]
#[case::drops_prefixed_cors(Operation::Filter, &[("llm_provider-access-control-allow-origin", "*")], &[])]
#[case::keeps_other_prefixed_header(
    Operation::Filter,
    &[("llm_provider-x-request-id", "abc")],
    &[("llm_provider-x-request-id", "abc")]
)]
#[case::keeps_prefixed_framing_header(
    Operation::Filter,
    &[("llm_provider-content-length", "12")],
    &[("llm_provider-content-length", "12")]
)]
#[case::keeps_bare_rate_limit_header(
    Operation::Filter,
    &[("x-ratelimit-limit-requests", "100")],
    &[("x-ratelimit-limit-requests", "100")]
)]
#[case::projects_unprefixed_header(
    Operation::Project,
    &[("x-request-id", "abc")],
    &[("llm_provider-x-request-id", "abc")]
)]
#[case::does_not_double_prefix(
    Operation::Project,
    &[("llm_provider-x-request-id", "abc")],
    &[("llm_provider-x-request-id", "abc")]
)]
#[case::projects_bare_rate_limit_headers_alongside_the_prefixed_copy(
    Operation::Project,
    &[("x-ratelimit-limit-requests", "100")],
    &[("x-ratelimit-limit-requests", "100"), ("llm_provider-x-ratelimit-limit-requests", "100")]
)]
#[case::response_headers_are_projected_then_filtered(
    Operation::AllowedFromResponse,
    &[("set-cookie", "a=b"), ("x-request-id", "abc")],
    &[("llm_provider-x-request-id", "abc")]
)]
#[case::explicit_headers_bypass_projection(
    Operation::AllowedFromExplicit,
    &[("retry-after", "5"), ("set-cookie", "a=b")],
    &[("retry-after", "5")]
)]
fn error_types_error_type_tests_behavior(
    #[case] operation: Operation,
    #[case] input: &[(&str, &str)],
    #[case] expected: &[(&str, &str)],
) {
    let actual = match operation {
        Operation::Filter => filter_unsafe_headers(map(input)),
        Operation::Project => project_provider_headers(&map(input)),
        Operation::AllowedFromResponse => allowed_response_headers(None, &map(input)),
        Operation::AllowedFromExplicit => {
            allowed_response_headers(Some(&map(input)), &BTreeMap::new())
        }
    };
    assert_eq!(actual, map(expected));
}

#[test]
fn missing_wire_field_serializes_as_the_literal_none_string() {
    assert_eq!(serde_json::to_value(WireField::Missing).unwrap(), "None");
}

#[test]
fn null_wire_field_serializes_as_json_null() {
    assert_eq!(
        serde_json::to_value(WireField::Null).unwrap(),
        serde_json::Value::Null
    );
}

#[test]
fn text_wire_field_serializes_as_the_string_it_holds() {
    assert_eq!(
        serde_json::to_value(WireField::text("throttling_error".to_owned())).unwrap(),
        "throttling_error"
    );
}
