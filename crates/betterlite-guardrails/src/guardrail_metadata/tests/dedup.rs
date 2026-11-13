use rstest::rstest;

use super::support::{apply_calls, assert_json, data};

#[rstest]
#[case::same_stage_twice("pre|scan-1|pii;pre|scan-1|pii",r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::the_same_id_at_two_stages("pre|scan-1|pii;post|scan-1|pii",r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}, {"guardrail": "pii", "stage": "post_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::stages_keep_first_seen_order("post|scan-1|pii;pre|scan-1|pii",r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "post_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}, {"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::different_guardrails_share_the_id("pre|scan-1|pii;pre|scan-1|moderation",r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}, {"guardrail": "moderation", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::different_ids_keep_first_seen_order("post|scan-2|pii;pre|scan-1|pii;post|scan-2|pii",r#"{"metadata": {"guardrail_scan_ids": ["scan-2", "scan-1"], "guardrail_scan_metadata": [{"guardrail": "pii", "stage": "post_call", "provider": "panw_prisma_airs", "scan_id": "scan-2"}, {"guardrail": "pii", "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
#[case::a_null_guardrail_is_kept("pre|scan-1|-;pre|scan-1|-",r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": null, "stage": "pre_call", "provider": "panw_prisma_airs", "scan_id": "scan-1"}]}}"#)]
fn guardrail_metadata_cases(#[case] script: &str, #[case] expected: &str) {
    let mut request_data = data("{}");
    apply_calls(&mut request_data, script);
    assert_json(&request_data, expected);
}
