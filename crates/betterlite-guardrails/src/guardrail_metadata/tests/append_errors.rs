use rstest::rstest;

use super::support::{assert_json, data, record};
use crate::guardrail_metadata::append_guardrail_response;

#[rstest]
#[case::the_standard_logging_object_must_be_an_object(r#"{"standard_logging_object": "text"}"#)]
#[case::guardrail_information_must_be_a_list(
    r#"{"standard_logging_object": {"guardrail_information": {"guardrail_name": "earlier"}}}"#
)]
fn guardrail_metadata_cases(#[case] start: &str) {
    let mut details = data(start);
    append_guardrail_response(Some(&mut details), &record()).unwrap_err();
    assert_json(&details, start);
}
