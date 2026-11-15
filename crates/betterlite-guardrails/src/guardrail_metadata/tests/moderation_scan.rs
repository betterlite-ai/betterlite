use rstest::rstest;

use super::support::{assert_json, data, moderation_records};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_metadata::merge_moderation_scan;

#[rstest]
#[case::a_response_scan_records_post_call("mod|scan-1",0,"response",Some("moderation"),r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "moderation", "stage": "post_call", "provider": "openai_moderation", "scan_id": "scan-1"}]}}"#)]
#[case::a_request_scan_records_pre_call("mod|scan-1",0,"request",Some("moderation"),r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "moderation", "stage": "pre_call", "provider": "openai_moderation", "scan_id": "scan-1"}]}}"#)]
#[case::records_before_the_start_index_are_not_reread("mod|scan-old,mod|scan-new",1,"response",Some("moderation"),r#"{"metadata": {"guardrail_scan_ids": ["scan-new"], "guardrail_scan_metadata": [{"guardrail": "moderation", "stage": "post_call", "provider": "openai_moderation", "scan_id": "scan-new"}]}}"#)]
#[case::a_failed_transport_record_carries_no_id(
    "text|unused",
    0,
    "response",
    Some("moderation"),
    "{}"
)]
#[case::a_flagged_moderation_result_still_records("flagged|scan-1",0,"response",Some("moderation"),r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": "moderation", "stage": "post_call", "provider": "openai_moderation", "scan_id": "scan-1"}]}}"#)]
#[case::a_null_guardrail_name_is_kept("mod|scan-1",0,"response",None,r#"{"metadata": {"guardrail_scan_ids": ["scan-1"], "guardrail_scan_metadata": [{"guardrail": null, "stage": "post_call", "provider": "openai_moderation", "scan_id": "scan-1"}]}}"#)]
#[case::every_new_moderation_record_is_read("mod|scan-1,mod|scan-2",0,"response",Some("moderation"),r#"{"metadata": {"guardrail_scan_ids": ["scan-1", "scan-2"], "guardrail_scan_metadata": [{"guardrail": "moderation", "stage": "post_call", "provider": "openai_moderation", "scan_id": "scan-1"}, {"guardrail": "moderation", "stage": "post_call", "provider": "openai_moderation", "scan_id": "scan-2"}]}}"#)]
fn guardrail_metadata_cases(
    #[case] script: &str,
    #[case] start_index: usize,
    #[case] input: &str,
    #[case] guardrail: Option<&str>,
    #[case] expected: &str,
) {
    let input_type = match input {
        "request" => GuardrailInputType::Request,
        _ => GuardrailInputType::Response,
    };
    let mut request_data = data("{}");
    merge_moderation_scan(
        &mut request_data,
        &moderation_records(script),
        start_index,
        guardrail,
        input_type,
    );
    assert_json(&request_data, expected);
}
