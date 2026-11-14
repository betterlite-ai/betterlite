use rstest::rstest;

use super::support::data;
use crate::guardrail_metadata::logging_caching_headers;

#[rstest]
#[case::a_pillar_header_value_must_be_a_string(r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_response_headers": {"x-pillar-a": true}}}"#)]
#[case::the_fallback_flag_must_be_a_boolean(
    r#"{"metadata": {"_pillar_response_headers_trusted": true, "pillar_flagged": "yes"}}"#
)]
fn guardrail_metadata_cases(#[case] request: &str) {
    logging_caching_headers(&data(request)).unwrap_err();
}
