use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::support::{
    self, APPLIED, FakeSelection, LogCapture, array_of, context, data, field, names_in, object_of,
    records, scan_entries, text_of,
};
use crate::panw_prisma_airs::audit::GuardrailAudit;
use crate::panw_prisma_airs::error_detail::{ErrorCode, ErrorType};
use crate::panw_prisma_airs::hooks::audit_bridge;
use crate::panw_prisma_airs::hooks::gate;
use crate::panw_prisma_airs::hooks::pre_call::pre_call_hook;
use crate::panw_prisma_airs::verdict::{Action, ScanVerdict};

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const BLOCK: &str = r#"{"action":"block","category":"malicious","scan_id":"scan-2"}"#;
const USER_REQUEST: &str =
    r#"{"litellm_call_id":"call-1","model":"gpt","messages":[{"role":"user","content":"hello"}]}"#;

#[tokio::test]
async fn allow_scan_records_the_scan_and_keeps_the_request() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(USER_REQUEST);
    let messages_before = request_data.get("messages").cloned().expect("messages");
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("allowed");
    assert_eq!(request_data.get("messages"), Some(&messages_before));
    assert_eq!(
        support::bucket_field(&request_data, "_panw_pre_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(names_in(&request_data, APPLIED), vec!["panw".to_owned()]);
    let entries = scan_entries(&request_data);
    let entry = entries.first().expect("scan entry");
    assert_eq!(entry.guardrail.as_deref(), Some("panw"));
    assert_eq!(entry.stage, GuardrailEventHooks::PreCall);
    assert_eq!(entry.provider, "panw_prisma_airs");
    assert_eq!(entry.scan_id, "scan-1");
}

#[tokio::test]
async fn allow_scan_logs_the_guardrail_record() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(USER_REQUEST);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("allowed");
    let record = records(&request_data).first().expect("record");
    assert_eq!(text_of(field(record, "guardrail_name")), "panw");
    let provider = text_of(field(record, "guardrail_provider"));
    assert_eq!(provider, "panw_prisma_airs");
    assert_eq!(text_of(field(record, "guardrail_mode")), "pre_call");
    assert_eq!(text_of(field(record, "guardrail_status")), "success");
    let start = field(record, "start_time");
    assert_eq!(start, &GuardrailValue::Decimal(12.0));
    assert_eq!(field(record, "end_time"), &GuardrailValue::Decimal(12.0));
    assert_eq!(field(record, "duration"), &GuardrailValue::Decimal(0.0));
    assert_eq!(field(record, "masked_entity_count"), &GuardrailValue::Null);
    let logged = field(record, "guardrail_response");
    assert_eq!(text_of(field(logged, "action")), "allow");
    assert_eq!(text_of(field(logged, "category")), "benign");
    assert_eq!(text_of(field(logged, "scan_id")), "scan-1");
}

#[tokio::test]
async fn allow_scan_sends_the_prompt_and_metadata() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(USER_REQUEST);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("allowed");
    let body = support::only_scan_body(&server).await;
    assert_eq!(text_of(field(&body, "tr_id")), "call-1");
    assert_eq!(support::scanned_text(&body, "prompt"), "hello");
    let sent = object_of(field(&body, "metadata"));
    assert_eq!(text_of(sent.get("ai_model").expect("model")), "gpt");
    assert_eq!(text_of(sent.get("app_user").expect("user")), "litellm_user");
    assert_eq!(text_of(sent.get("app_name").expect("app")), "LiteLLM");
    assert!(sent.get("is_response").is_none());
    let profile = field(&body, "ai_profile");
    assert_eq!(text_of(field(profile, "profile_name")), "config-profile");
}

#[tokio::test]
async fn block_scan_returns_the_detailed_400_and_logs_the_intervention() {
    let server = support::serve(StatusCode::OK, BLOCK).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(USER_REQUEST);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect_err("blocked");
    assert_eq!(error.status, 400);
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsBlocked);
    assert_eq!(error.detail.error.kind, ErrorType::GuardrailViolation);
    assert_eq!(error.detail.error.action, Some(Action::Block));
    assert_eq!(error.detail.error.category, "malicious");
    assert_eq!(error.detail.error.guardrail, "panw");
    assert_eq!(
        error.detail.error.message,
        "Prompt blocked by PANW Prisma AI Security policy (Category: malicious)"
    );
    let scan_id = error.detail.error.scan_fields.get("scan_id").expect("scan");
    assert_eq!(scan_id.get(), "\"scan-2\"");
    let record = records(&request_data).first().expect("record");
    assert_eq!(
        text_of(field(record, "guardrail_status")),
        "guardrail_intervened"
    );
    assert_eq!(
        text_of(field(field(record, "guardrail_response"), "action")),
        "block"
    );
    assert_eq!(names_in(&request_data, APPLIED), Vec::<String>::new());
    assert_eq!(
        names_in(&request_data, "guardrail_scan_ids"),
        vec!["scan-2".to_owned()]
    );
}

#[tokio::test]
async fn allow_with_masked_text_rewrites_the_last_user_message() {
    let logs = LogCapture::start("pre-mask", tracing::Level::INFO);
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"allow","category":"benign","prompt_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(
        r#"{"litellm_call_id":"call-1","messages":[{"role":"system","content":"sys"},{"role":"user","name":"first","content":"one"},{"role":"user","content":"two"}]}"#,
    );
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("allowed");
    let messages = array_of(request_data.get("messages").expect("messages"));
    assert_eq!(text_of(field(&messages[1], "name")), "first");
    assert_eq!(text_of(field(&messages[1], "content")), "one");
    assert_eq!(text_of(field(&messages[2], "content")), "MASKED");
    assert_eq!(names_in(&request_data, APPLIED), vec!["panw".to_owned()]);
    let record = records(&request_data).first().expect("record");
    assert_eq!(text_of(field(record, "guardrail_status")), "success");
    let written = logs.finish();
    assert!(written.contains("PANW Prisma AIRS: Prompt allowed with masking (Category: benign)"));
}

#[tokio::test]
async fn allow_with_masked_text_but_no_eligible_message_fails_closed() {
    let logs = LogCapture::start("pre-mask-no-message", tracing::Level::INFO);
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"allow","category":"benign","prompt_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::blocking(&server.base);
    let mut request_data =
        data(r#"{"litellm_call_id":"call-1","messages":[{"role":"system","content":"sys"}]}"#);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect_err("cannot mask");
    assert_eq!(error.status, 500);
    assert_eq!(error.detail.error.category, "api_error");
    let written = logs.finish();
    assert!(written.contains("no user or developer message has content to mask"));
}

#[tokio::test]
async fn allow_with_masked_text_keeps_non_text_parts() {
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"allow","category":"benign","prompt_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(
        r#"{"litellm_call_id":"call-1","messages":[{"role":"user","content":[{"type":"text","text":"hello"},{"type":"image_url","image_url":{"url":"u"}}]}]}"#,
    );
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("allowed");
    let messages = array_of(request_data.get("messages").expect("messages"));
    let parts = array_of(field(&messages[0], "content"));
    assert_eq!(parts.len(), 2);
    assert_eq!(text_of(field(&parts[0], "type")), "text");
    assert_eq!(text_of(field(&parts[0], "text")), "MASKED");
    assert_eq!(text_of(field(&parts[1], "type")), "image_url");
    assert_eq!(text_of(field(field(&parts[1], "image_url"), "url")), "u");
}

#[tokio::test]
async fn allow_with_masked_text_rewrites_the_prompt_field() {
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"allow","category":"benign","prompt_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1","prompt":"hi"}"#);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("allowed");
    assert_eq!(
        request_data.get("prompt"),
        Some(&GuardrailValue::Text("MASKED".to_owned()))
    );
}

#[tokio::test]
async fn block_with_a_mask_and_the_request_flag_masks_instead() {
    let logs = LogCapture::start("pre-block-mask", tracing::Level::INFO);
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"block","category":"malicious","prompt_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::masking_requests(&server.base);
    let mut request_data = data(USER_REQUEST);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("masked");
    let messages = array_of(request_data.get("messages").expect("messages"));
    assert_eq!(text_of(field(&messages[0], "content")), "MASKED");
    assert_eq!(names_in(&request_data, APPLIED), vec!["panw".to_owned()]);
    let record = records(&request_data).first().expect("record");
    assert_eq!(
        text_of(field(record, "guardrail_status")),
        "guardrail_intervened"
    );
    let written = logs.finish();
    assert!(written.contains("Prompt blocked but masked instead (mask_request_content=True)"));
}

#[tokio::test]
async fn block_with_a_mask_but_no_request_flag_still_blocks() {
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"block","category":"malicious","prompt_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(USER_REQUEST);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect_err("blocked");
    assert_eq!(error.status, 400);
    let messages = array_of(request_data.get("messages").expect("messages"));
    assert_eq!(text_of(field(&messages[0], "content")), "hello");
    assert_eq!(names_in(&request_data, APPLIED), Vec::<String>::new());
}

#[tokio::test]
async fn transient_error_with_fallback_allow_passes_the_request_through() {
    let logs = LogCapture::start("pre-transient", tracing::Level::INFO);
    let server = support::serve(StatusCode::TOO_MANY_REQUESTS, "{}").await;
    let guardrail = support::allowing(&server.base);
    let mut request_data = data(USER_REQUEST);
    let messages_before = request_data.get("messages").cloned().expect("messages");
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("unscanned");
    assert_eq!(request_data.get("messages"), Some(&messages_before));
    assert_eq!(
        names_in(&request_data, APPLIED),
        vec!["panw:unscanned".to_owned()]
    );
    let record = records(&request_data).first().expect("record");
    assert_eq!(
        text_of(field(record, "guardrail_status")),
        "guardrail_failed_to_respond"
    );
    assert_eq!(
        text_of(field(field(record, "guardrail_response"), "category")),
        "http_429_error"
    );
    let written = logs.finish();
    assert!(written.contains(
        "PANW Prisma AIRS: Allowing request without scanning (fallback_on_error='allow', error: http_429_error)"
    ));
}

#[tokio::test]
async fn transient_error_with_fallback_block_returns_a_500() {
    let server = support::serve(StatusCode::TOO_MANY_REQUESTS, "{}").await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(USER_REQUEST);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect_err("blocked");
    assert_eq!(error.status, 500);
    assert_eq!(
        error.detail.error.message,
        "Security scan failed - request blocked for safety"
    );
    assert_eq!(error.detail.error.kind, ErrorType::GuardrailScanError);
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsScanFailed);
    assert_eq!(error.detail.error.category, "http_429_error");
    assert_eq!(error.detail.error.action, None);
}

#[tokio::test]
async fn permanent_client_error_returns_a_500_scan_failure() {
    let server = support::serve(StatusCode::BAD_REQUEST, "{}").await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(USER_REQUEST);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect_err("blocked");
    assert_eq!(error.status, 500);
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsScanFailed);
    assert_eq!(error.detail.error.category, "http_400_error");
    let record = records(&request_data).first().expect("record");
    let logged = field(record, "guardrail_response");
    assert_eq!(
        field(logged, "_always_block"),
        &GuardrailValue::Boolean(true)
    );
}

#[tokio::test]
async fn authentication_error_returns_the_configuration_failure() {
    let server = support::serve(StatusCode::UNAUTHORIZED, "{}").await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(USER_REQUEST);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect_err("blocked");
    assert_eq!(error.status, 500);
    assert_eq!(
        error.detail.error.code,
        ErrorCode::PanwPrismaAirsConfigError
    );
    assert_eq!(error.detail.error.kind, ErrorType::GuardrailConfigError);
    assert_eq!(
        error.detail.error.message,
        "Security scan failed - configuration error"
    );
    assert_eq!(error.detail.error.category, "config_error");
}

#[tokio::test]
async fn a_suppressed_guardrail_leaves_the_request_untouched() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(USER_REQUEST);
    let original = request_data.clone();
    let selection = FakeSelection { events: vec![] };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("pass through");
    assert_eq!(request_data, original);
    assert!(server.body_texts().await.is_empty());
}

#[tokio::test]
async fn an_empty_prompt_warns_and_skips_the_scan() {
    let logs = LogCapture::start("pre-empty", tracing::Level::INFO);
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect("skipped");
    assert!(server.body_texts().await.is_empty());
    assert_eq!(
        support::bucket_field(&request_data, "_panw_pre_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
    let written = logs.finish();
    assert!(written.contains(
        "PANW Prisma AIRS: No user prompt found in request (checked 'messages' and 'prompt' fields)"
    ));
}

#[tokio::test]
async fn malformed_messages_become_the_catch_all_500() {
    let logs = LogCapture::start("pre-malformed", tracing::Level::INFO);
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1","messages":[{"role":10}]}"#);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PreCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = pre_call_hook(&guardrail, &mut request_data, &mut audit, &hook_context)
        .await
        .expect_err("failed");
    assert_eq!(error.status, 500);
    assert_eq!(
        error.detail.error.message,
        "Security scan failed - request blocked for safety"
    );
    assert_eq!(error.detail.error.kind, ErrorType::GuardrailScanError);
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsScanFailed);
    assert_eq!(error.detail.error.category, "api_error");
    assert_eq!(error.detail.error.action, None);
    let written = logs.finish();
    assert!(written.contains("PANW Prisma AIRS scan failed: item 0 of messages"));
}

#[rstest::rstest]
#[case::pre_mcp_falls_back_to_pre_call(
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::PreCall,
    true
)]
#[case::during_mcp_falls_back_to_during_call(
    GuardrailEventHooks::DuringMcpCall,
    GuardrailEventHooks::DuringCall,
    true
)]
#[case::pre_mcp_ignores_a_post_call_selection(
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::PostCall,
    false
)]
#[case::post_mcp_has_no_legacy_hook(
    GuardrailEventHooks::PostMcpCall,
    GuardrailEventHooks::PostCall,
    false
)]
#[case::pre_call_runs_directly(GuardrailEventHooks::PreCall, GuardrailEventHooks::PreCall, true)]
#[case::post_call_runs_directly(GuardrailEventHooks::PostCall, GuardrailEventHooks::PostCall, true)]
#[case::logging_only_has_no_legacy_hook(
    GuardrailEventHooks::LoggingOnly,
    GuardrailEventHooks::PreCall,
    false
)]
#[case::transcription_has_no_legacy_hook(
    GuardrailEventHooks::RealtimeInputTranscription,
    GuardrailEventHooks::PreCall,
    false
)]
fn mcp_events_fall_back_to_the_legacy_hook(
    #[case] event: GuardrailEventHooks,
    #[case] selected: GuardrailEventHooks,
    #[case] expected: bool,
) {
    let selection = FakeSelection {
        events: vec![selected],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let request_data = data("{}");
    assert_eq!(
        gate::should_run_guardrail(&request_data, event, &hook_context).expect("gate"),
        expected
    );
}

#[tokio::test]
async fn a_start_before_the_epoch_is_logged_as_zero() {
    let guardrail = support::blocking("http://127.0.0.1:1");
    let mut request_data = data(r#"{"metadata":{"standard_logging_guardrail_information":null}}"#);
    let mut audit = GuardrailAudit::default();
    let before_epoch = SystemTime::UNIX_EPOCH
        .checked_sub(Duration::from_secs(1))
        .expect("time in range");
    let verdict = ScanVerdict::synthetic(Action::Allow, "benign", None);
    audit_bridge::record_response_success(&guardrail, &mut audit, &verdict, before_epoch);
    audit_bridge::flush(&mut request_data, &audit, "panw");
    let record = records(&request_data).first().expect("record");
    assert_eq!(field(record, "start_time"), &GuardrailValue::Decimal(0.0));
    assert_eq!(field(record, "duration"), &GuardrailValue::Decimal(13.0));
    assert_eq!(text_of(field(record, "guardrail_mode")), "post_call");
}

#[tokio::test]
async fn flushing_twice_keeps_one_record_and_an_earlier_value() {
    let guardrail = support::blocking("http://127.0.0.1:1");
    let mut request_data = data(r#"{"metadata":{"standard_logging_guardrail_information":"old"}}"#);
    let mut audit = GuardrailAudit::default();
    let verdict = ScanVerdict::synthetic(Action::Allow, "benign", None);
    audit_bridge::record_response_success(&guardrail, &mut audit, &verdict, support::now());
    audit_bridge::flush(&mut request_data, &audit, "panw");
    audit_bridge::flush(&mut request_data, &audit, "panw");
    let entries = records(&request_data);
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries.first(),
        Some(&GuardrailValue::Text("old".to_owned()))
    );
}
