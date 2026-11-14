use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rstest::rstest;

use super::support::{assert_json, data};
use crate::guardrail_metadata::record_scan_id;

#[rstest]
#[case::creates_the_metadata_bucket("{}",Some("scan-1"),r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::keeps_the_rest_of_the_metadata_bucket(r#"{"metadata": {"keep": 1}}"#,Some("scan-1"),r#"{"metadata": {"keep": 1, "guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::selects_litellm_metadata_when_the_key_exists(r#"{"metadata": {"guardrail_scan_ids": ["old"]}, "litellm_metadata": {}}"#,Some("scan-1"),r#"{"metadata": {"guardrail_scan_ids": ["old"]}, "litellm_metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::replaces_a_null_litellm_metadata(r#"{"metadata": {"keep": 1}, "litellm_metadata": null}"#,Some("scan-1"),r#"{"metadata": {"keep": 1}, "litellm_metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::replaces_a_non_object_bucket(r#"{"metadata": "text"}"#,Some("scan-1"),r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::records_a_whitespace_id("{}",Some(" "),r#"{"metadata": {"guardrail_scan_ids": [" "], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": " "}]}}"#)]
#[case::absent_id_is_a_no_op("{}", None, "{}")]
#[case::empty_id_is_a_no_op("{}", Some(""), "{}")]
#[case::empty_id_does_not_create_a_bucket(r#"{"keep": 1}"#, Some(""), r#"{"keep": 1}"#)]
#[case::a_non_list_ids_value_is_replaced(r#"{"metadata": {"guardrail_scan_ids": "scan-old"}}"#,Some("scan-1"),r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::an_entry_with_an_extra_key_is_not_the_same_entry(r#"{"metadata": {"guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1", "extra": true}]}}"#,Some("scan-1"),r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1", "extra": true}, {"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
fn guardrail_metadata_cases(
    #[case] start: &str,
    #[case] scan_id: Option<&str>,
    #[case] expected: &str,
) {
    let mut request_data = data(start);
    record_scan_id(
        &mut request_data,
        scan_id,
        Some("pii"),
        "panw_prisma_airs",
        GuardrailEventHooks::PreCall,
    );
    assert_json(&request_data, expected);
}
