use rstest::rstest;

use super::support::data;
use crate::guardrail_metadata::logging_caching_headers;

#[rstest]
#[case::applied_guardrails_must_be_a_list_of_strings(
    r#"{"metadata": {"applied_guardrails": "pii"}}"#
)]
#[case::applied_policy_entries_must_be_strings(r#"{"metadata": {"applied_policies": ["ok", 2]}}"#)]
#[case::scan_ids_must_be_a_list_of_strings(r#"{"metadata": {"guardrail_scan_ids": 7}}"#)]
#[case::policy_sources_must_be_a_map(r#"{"metadata": {"policy_sources": ["p1"]}}"#)]
#[case::policy_source_reasons_must_be_strings(r#"{"metadata": {"policy_sources": {"p1": 1}}}"#)]
#[case::semantic_similarity_must_be_a_number(r#"{"metadata": {"semantic-similarity": "high"}}"#)]
fn guardrail_metadata_cases(#[case] request: &str) {
    logging_caching_headers(&data(request)).unwrap_err();
}
