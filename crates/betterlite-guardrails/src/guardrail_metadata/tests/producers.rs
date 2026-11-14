use rstest::rstest;

use super::support::{assert_json, audit, data};
use crate::guardrail_metadata::merge_prisma_audit;

#[rstest]
#[case::both_stages_are_recorded("pre|scan-1|pii,post|scan-1|pii",r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}, {"guardrail": "pii", "stage": "post_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::an_empty_audit_leaves_the_request_alone("", "{}")]
#[case::duplicate_audit_entries_are_deduplicated("pre|scan-1|pii,pre|scan-1|pii",r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::audit_order_is_kept("post|scan-2|pii,pre|scan-1|pii",r#"{"metadata": {"guardrail_scan_ids": ["scan-2", "scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "post_call", "provider": "panw_prisma_airs", "scan_id": "scan-2"}, {"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
fn guardrail_metadata_cases(#[case] script: &str, #[case] expected: &str) {
    let mut request_data = data("{}");
    merge_prisma_audit(&mut request_data, &audit(script));
    assert_json(&request_data, expected);
}
