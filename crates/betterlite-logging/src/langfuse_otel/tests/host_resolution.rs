#![cfg(test)]

use rstest::rstest;

use super::environment;
use crate::langfuse_otel::destination::langfuse_otel_host;

#[rstest]
#[case::otel_host_wins(&[("LANGFUSE_OTEL_HOST", "otel.example"), ("LANGFUSE_HOST", "plain.example")], Some("otel.example"))]
#[case::langfuse_host_fallback(&[("LANGFUSE_HOST", "plain.example")], Some("plain.example"))]
#[case::otel_host_alone(&[("LANGFUSE_OTEL_HOST", "otel.example")], Some("otel.example"))]
#[case::neither_is_set(&[], None)]
#[case::empty_otel_host_is_unset(&[("LANGFUSE_OTEL_HOST", ""), ("LANGFUSE_HOST", "plain.example")], Some("plain.example"))]
#[case::both_empty(&[("LANGFUSE_OTEL_HOST", ""), ("LANGFUSE_HOST", "")], None)]
fn host_resolution_follows_the_precedence(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        langfuse_otel_host(&environment(variables)).as_deref(),
        expected
    );
}
