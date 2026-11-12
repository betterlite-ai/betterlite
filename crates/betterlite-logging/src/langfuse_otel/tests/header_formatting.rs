#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;

use crate::langfuse_otel::destination::{OtelHeaders, format_otel_headers};

fn headers(entries: &[(&'static str, &str)]) -> OtelHeaders {
    entries
        .iter()
        .map(|(name, value)| (*name, SecretString::from(*value)))
        .collect()
}

#[rstest]
#[case::langfuse_pair(
    &[("Authorization", "Basic abc"), ("x-langfuse-ingestion-version", "4")],
    "Authorization=Basic abc,x-langfuse-ingestion-version=4"
)]
#[case::insertion_order_is_kept(&[("b", "2"), ("a", "1")], "b=2,a=1")]
#[case::single_header(&[("a", "1")], "a=1")]
#[case::no_headers(&[], "")]
#[case::values_are_not_escaped(&[("a", "x=y,z")], "a=x=y,z")]
fn header_formatting_joins_key_value_pairs_with_commas(
    #[case] entries: &[(&'static str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(format_otel_headers(&headers(entries)), expected);
}
