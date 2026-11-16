use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::support::{self, APPLIED, FakeSelection, LogCapture, context, data, names_in, response};
use crate::panw_prisma_airs::audit::GuardrailAudit;
use crate::panw_prisma_airs::hooks::dedup::{ScanType, check_and_mark_scanned};
use crate::panw_prisma_airs::hooks::post_call::post_call_hook;
use crate::panw_prisma_airs::hooks::pre_call::pre_call_hook;
use crate::panw_prisma_airs::hooks::response::CallResponse;

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const ANSWER: &str = r#"{"choices":[{"message":{"content":"answer"}}]}"#;
const SYNTHESIZED: &str = "00000000-0000-0000-0000-000000000007";

#[test]
fn pre_post_and_streaming_markers_stay_separate() {
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    assert!(
        !check_and_mark_scanned(&mut request_data, ScanType::Pre, support::new_id).expect("first")
    );
    assert!(
        check_and_mark_scanned(&mut request_data, ScanType::Pre, support::new_id).expect("second")
    );
    assert!(
        !check_and_mark_scanned(&mut request_data, ScanType::Post, support::new_id).expect("post")
    );
    assert!(
        !check_and_mark_scanned(&mut request_data, ScanType::Streaming, support::new_id)
            .expect("streaming")
    );
    assert_eq!(
        support::bucket_field(&request_data, "_panw_pre_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(
        support::bucket_field(&request_data, "_panw_post_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(
        support::bucket_field(&request_data, "_panw_streaming_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[test]
fn a_missing_call_id_is_synthesized_and_warned() {
    let logs = LogCapture::start("dedup-synth", tracing::Level::INFO);
    let mut request_data = data("{}");
    assert!(
        !check_and_mark_scanned(&mut request_data, ScanType::Pre, support::new_id).expect("first")
    );
    assert_eq!(
        request_data.get("litellm_call_id"),
        Some(&GuardrailValue::Text(SYNTHESIZED.to_owned()))
    );
    assert_eq!(
        support::bucket_field(&request_data, &format!("_panw_pre_scanned_{SYNTHESIZED}")),
        Some(&GuardrailValue::Boolean(true))
    );
    assert!(
        check_and_mark_scanned(&mut request_data, ScanType::Pre, support::new_id).expect("second")
    );
    let written = logs.finish();
    let warnings = written.matches("synthesized").count();
    assert_eq!(warnings, 1);
    assert!(written.contains(&format!(
        "synthesized {SYNTHESIZED} for pre scan deduplication"
    )));
}

#[test]
fn an_empty_call_id_is_synthesized() {
    let mut request_data = data(r#"{"litellm_call_id":""}"#);
    assert!(
        !check_and_mark_scanned(&mut request_data, ScanType::Post, support::new_id).expect("first")
    );
    assert_eq!(
        request_data.get("litellm_call_id"),
        Some(&GuardrailValue::Text(SYNTHESIZED.to_owned()))
    );
    assert_eq!(
        support::bucket_field(&request_data, &format!("_panw_post_scanned_{SYNTHESIZED}")),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[test]
fn a_true_marker_skips_the_scan() {
    let logs = LogCapture::start("dedup-skip", tracing::Level::DEBUG);
    let mut request_data = data(
        r#"{"litellm_call_id":"call-1","litellm_metadata":{"_panw_pre_scanned_call-1":true}}"#,
    );
    assert!(
        check_and_mark_scanned(&mut request_data, ScanType::Pre, support::new_id).expect("skip")
    );
    let written = logs.finish();
    assert!(written.contains("PANW Prisma AIRS: Skipping duplicate pre-call scan"));
}

#[test]
fn a_false_marker_scans_again() {
    let mut request_data = data(
        r#"{"litellm_call_id":"call-1","litellm_metadata":{"_panw_pre_scanned_call-1":false}}"#,
    );
    assert!(
        !check_and_mark_scanned(&mut request_data, ScanType::Pre, support::new_id).expect("scan")
    );
    assert_eq!(
        support::bucket_field(&request_data, "_panw_pre_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[test]
fn a_non_boolean_marker_names_the_marker() {
    let mut request_data = data(
        r#"{"litellm_call_id":"call-1","litellm_metadata":{"_panw_pre_scanned_call-1":"yes"}}"#,
    );
    let error = check_and_mark_scanned(&mut request_data, ScanType::Pre, support::new_id)
        .expect_err("marker");
    assert_eq!(
        error.to_string(),
        "the scan marker _panw_pre_scanned_call-1 must be a boolean"
    );
}

#[test]
fn a_non_object_metadata_names_the_field() {
    let mut request_data = data(r#"{"litellm_call_id":"call-1","litellm_metadata":"x"}"#);
    let error = check_and_mark_scanned(&mut request_data, ScanType::Pre, support::new_id)
        .expect_err("metadata");
    assert_eq!(
        error.to_string(),
        "the litellm_metadata field must be an object"
    );
}

#[test]
fn a_null_metadata_is_treated_as_absent() {
    let mut request_data = data(r#"{"litellm_call_id":"call-1","litellm_metadata":null}"#);
    assert!(
        !check_and_mark_scanned(&mut request_data, ScanType::Pre, support::new_id).expect("scan")
    );
    assert_eq!(
        support::bucket_field(&request_data, "_panw_pre_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[tokio::test]
async fn each_hook_scans_once_per_call_id() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data =
        data(r#"{"litellm_call_id":"call-1","messages":[{"role":"user","content":"hello"}]}"#);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("pre");
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("pre again");
    let mut model_response = response(ANSWER);
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("post");
    assert_eq!(server.body_texts().await.len(), 2);
    assert_eq!(
        support::bucket_field(&request_data, "_panw_pre_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(
        support::bucket_field(&request_data, "_panw_post_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(names_in(&request_data, APPLIED), vec!["panw".to_owned()]);
    assert_eq!(
        names_in(&request_data, "guardrail_scan_ids"),
        vec!["scan-1".to_owned()]
    );
}
