use super::object;

use crate::stream_chunks::fallback::include_fallback_errors;
use rstest::rstest;

#[rstest]
#[case::text(r#"{"include_fallback_errors": "true"}"#)]
#[case::number(r#"{"include_fallback_errors": 1}"#)]
#[case::null(r#"{"include_fallback_errors": null}"#)]
#[case::list(r#"{"include_fallback_errors": [true]}"#)]
#[case::false_value(r#"{"include_fallback_errors": false}"#)]
fn include_errors_requires_json_true(#[case] raw: &str) {
    assert!(!include_fallback_errors(true, &object(raw)), "{raw}");
}
