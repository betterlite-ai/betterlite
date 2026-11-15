use rstest::rstest;

use super::support::{assert_json, data, headers};

#[rstest]
#[case::a_scan_entry_becomes_the_metadata_header(r#"{"metadata": {"guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#,r#"[["x-litellm-guardrail-scan-metadata", "[{\"guardrail\":\"pii\",\"stage\":\"pre_call\",\"provider\":\"panw_prisma_airs\",\"scan_id\":\"scan-1\"}]"]]"#)]
#[case::an_empty_entry_list_emits_no_metadata_header(
    r#"{"metadata": {"guardrail_scan_metadata": []}}"#,
    "[]"
)]
#[case::a_non_list_entry_value_is_ignored(
    r#"{"metadata": {"guardrail_scan_metadata": {"scan_id": "scan-1"}}}"#,
    "[]"
)]
#[case::a_null_guardrail_is_encoded_as_null(r#"{"metadata": {"guardrail_scan_metadata": [{"guardrail": null, "stage": "post_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#,r#"[["x-litellm-guardrail-scan-metadata", "[{\"guardrail\":null,\"stage\":\"post_call\",\"provider\":\"panw_prisma_airs\",\"scan_id\":\"scan-1\"}]"]]"#)]
#[case::entries_keep_their_order(r#"{"metadata": {"guardrail_scan_metadata": [{"guardrail": "b", "stage": "post_call", "provider": "panw_prisma_airs", "scan_id": "scan-2"}, {"guardrail": "a", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#,r#"[["x-litellm-guardrail-scan-metadata", "[{\"guardrail\":\"b\",\"stage\":\"post_call\",\"provider\":\"panw_prisma_airs\",\"scan_id\":\"scan-2\"},{\"guardrail\":\"a\",\"stage\":\"pre_call\",\"provider\":\"panw_prisma_airs\",\"scan_id\":\"scan-1\"}]"]]"#)]
fn guardrail_metadata_cases(#[case] request: &str, #[case] expected: &str) {
    assert_json(&headers(&data(request)), expected);
}
