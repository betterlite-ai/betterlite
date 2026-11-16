use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::support::{
    self, APPLIED, FakeSelection, LogCapture, context, data, field, json, names_in, records,
    response, scan_entries, text_of,
};
use crate::panw_prisma_airs::audit::GuardrailAudit;
use crate::panw_prisma_airs::error_detail::{ErrorCode, ErrorType};
use crate::panw_prisma_airs::hooks::post_call::post_call_hook;
use crate::panw_prisma_airs::hooks::response::CallResponse;
use crate::panw_prisma_airs::verdict::Action;
use crate::prisma_airs_masking::MaskContent;

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const BLOCK: &str = r#"{"action":"block","category":"harmful","scan_id":"scan-2"}"#;
const TWO_ANSWERS: &str =
    r#"{"choices":[{"message":{"content":"a"}},{"message":{"content":"b"}}]}"#;
const ANSWER: &str = r#"{"choices":[{"message":{"content":"answer"}}]}"#;
const RICH: &str = r#"{"choices":[{"message":{"content":"a","tool_calls":[{"function":{"arguments":"{\"x\":1}"}}],"function_call":{"arguments":"orig"}}},{"message":{"content":[{"type":"text","text":"b"},5,{"type":"image_url","image_url":{"url":"u"}}]}}]}"#;

#[tokio::test]
async fn allow_scan_records_the_response_scan() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1","model":"gpt"}"#);
    let mut model_response = response(ANSWER);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("allowed");
    assert_eq!(
        support::bucket_field(&request_data, "_panw_post_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(names_in(&request_data, APPLIED), vec!["panw".to_owned()]);
    let entries = scan_entries(&request_data);
    let entry = entries.first().expect("scan entry");
    assert_eq!(entry.stage, GuardrailEventHooks::PostCall);
    assert_eq!(entry.scan_id, "scan-1");
    let record = records(&request_data).first().expect("record");
    assert_eq!(text_of(field(record, "guardrail_mode")), "post_call");
    assert_eq!(text_of(field(record, "guardrail_status")), "success");
    let message = support::message_at(&model_response, 0);
    assert_eq!(support::text_content(message), "answer");
}

#[tokio::test]
async fn allow_scan_sends_the_response_text_and_metadata() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1","model":"gpt"}"#);
    let mut model_response = response(ANSWER);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("allowed");
    let body = support::only_scan_body(&server).await;
    assert_eq!(text_of(field(&body, "tr_id")), "call-1");
    assert_eq!(support::scanned_text(&body, "response"), "answer");
    assert_eq!(
        field(field(&body, "metadata"), "is_response"),
        &GuardrailValue::Boolean(true)
    );
}

#[tokio::test]
async fn allow_with_masked_text_rewrites_content_and_parts() {
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"allow","category":"benign","response_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(RICH);
    support::run_post_call(&guardrail, &mut request_data, &mut model_response)
        .await
        .expect("allowed");
    let body = support::only_scan_body(&server).await;
    assert_eq!(
        support::scanned_text(&body, "response"),
        "a {\"x\":1} orig b"
    );
    let first = support::message_at(&model_response, 0);
    assert_eq!(support::text_content(first), "MASKED");
    let second = support::message_at(&model_response, 1);
    match second.content.as_ref() {
        Some(MaskContent::Parts(parts)) => {
            assert_eq!(parts.len(), 3);
            let text = json(parts.first().expect("text part").get());
            assert_eq!(text_of(field(&text, "text")), "MASKED");
            assert_eq!(json(parts.get(1).expect("raw part").get()), json("5"));
            let image = json(parts.get(2).expect("image part").get());
            assert_eq!(text_of(field(&image, "type")), "image_url");
            assert_eq!(text_of(field(field(&image, "image_url"), "url")), "u");
        }
        _ => panic!("parts content"),
    }
}

#[tokio::test]
async fn allow_with_masked_text_rewrites_call_arguments() {
    let logs = LogCapture::start("post-mask", tracing::Level::INFO);
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"allow","category":"benign","response_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(RICH);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("allowed");
    let first = support::message_at(&model_response, 0);
    let calls = first.tool_calls.as_ref().expect("tool calls");
    let call = calls.first().expect("tool call");
    let function = call.function.as_ref().expect("function");
    assert_eq!(function.arguments.as_deref(), Some("MASKED"));
    let function_call = first.function_call.as_ref().expect("function call");
    assert_eq!(function_call.arguments.as_deref(), Some("MASKED"));
    let written = logs.finish();
    assert!(written.contains("PANW Prisma AIRS: Response allowed with masking (Category: benign)"));
}

#[tokio::test]
async fn tool_calls_without_arguments_are_ignored() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(
        r#"{"choices":[{"message":{"content":"a","tool_calls":[{}],"function_call":{}}}]}"#,
    );
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("allowed");
    let body = support::only_scan_body(&server).await;
    assert_eq!(support::scanned_text(&body, "response"), "a");
}

#[tokio::test]
async fn block_returns_the_detailed_400_response_error() {
    let logs = LogCapture::start("post-block", tracing::Level::INFO);
    let server = support::serve(StatusCode::OK, BLOCK).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect_err("blocked");
    assert_eq!(error.status, 400);
    assert_eq!(
        error.detail.error.code,
        ErrorCode::PanwPrismaAirsResponseBlocked
    );
    assert_eq!(error.detail.error.action, Some(Action::Block));
    assert_eq!(error.detail.error.category, "harmful");
    let message = &error.detail.error.message;
    assert_eq!(
        message,
        "Response blocked by PANW Prisma AI Security policy (Category: harmful)"
    );
    let record = records(&request_data).first().expect("record");
    assert_eq!(
        text_of(field(record, "guardrail_status")),
        "guardrail_intervened"
    );
    assert_eq!(names_in(&request_data, APPLIED), Vec::<String>::new());
    let written = logs.finish();
    assert!(written.contains(message.as_str()));
}

#[tokio::test]
async fn block_with_a_mask_and_the_response_flag_masks_instead() {
    let logs = LogCapture::start("post-block-mask", tracing::Level::INFO);
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"block","category":"harmful","response_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::masking_responses(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("masked");
    let message = support::message_at(&model_response, 0);
    assert_eq!(support::text_content(message), "MASKED");
    assert_eq!(names_in(&request_data, APPLIED), vec!["panw".to_owned()]);
    let record = records(&request_data).first().expect("record");
    assert_eq!(
        text_of(field(record, "guardrail_status")),
        "guardrail_intervened"
    );
    let written = logs.finish();
    assert!(written.contains("Response blocked but masked instead (mask_response_content=True)"));
}

#[tokio::test]
async fn block_with_a_mask_but_no_response_flag_still_blocks() {
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"block","category":"harmful","response_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect_err("blocked");
    assert_eq!(error.status, 400);
    let message = support::message_at(&model_response, 0);
    assert_eq!(support::text_content(message), "answer");
}

#[tokio::test]
async fn a_non_model_response_passes_through_untouched() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let original = request_data.clone();
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Other,
        &mut audit,
        &hook_context,
    )
    .await
    .expect("pass through");
    assert_eq!(request_data, original);
    assert!(server.body_texts().await.is_empty());
}

#[tokio::test]
async fn an_empty_response_warns_and_skips_the_scan() {
    let logs = LogCapture::start("post-empty", tracing::Level::INFO);
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(r#"{"choices":[{"message":{"content":""}}]}"#);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("skipped");
    assert!(server.body_texts().await.is_empty());
    assert_eq!(
        support::bucket_field(&request_data, "_panw_post_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
    let written = logs.finish();
    assert!(written.contains("PANW Prisma AIRS: No response content found to scan"));
}

#[tokio::test]
async fn transient_error_with_fallback_allow_keeps_the_response() {
    let server = support::serve(StatusCode::TOO_MANY_REQUESTS, "{}").await;
    let guardrail = support::allowing(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("unscanned");
    assert_eq!(
        names_in(&request_data, APPLIED),
        vec!["panw:unscanned".to_owned()]
    );
    let record = records(&request_data).first().expect("record");
    assert_eq!(
        text_of(field(record, "guardrail_status")),
        "guardrail_failed_to_respond"
    );
    let logged = field(record, "guardrail_response");
    assert_eq!(
        field(logged, "_is_transient"),
        &GuardrailValue::Boolean(true)
    );
    assert_eq!(text_of(field(logged, "category")), "http_429_error");
    let message = support::message_at(&model_response, 0);
    assert_eq!(support::text_content(message), "answer");
}

#[tokio::test]
async fn transient_error_with_fallback_block_returns_a_500() {
    let server = support::serve(StatusCode::TOO_MANY_REQUESTS, "{}").await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect_err("blocked");
    assert_eq!(error.status, 500);
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsScanFailed);
    assert_eq!(error.detail.error.category, "http_429_error");
    assert_eq!(error.detail.error.kind, ErrorType::GuardrailScanError);
    let record = records(&request_data).first().expect("record");
    let logged = field(record, "guardrail_response");
    assert_eq!(
        field(logged, "_is_transient"),
        &GuardrailValue::Boolean(true)
    );
}

#[tokio::test]
async fn an_oversized_mask_becomes_the_catch_all_500() {
    let logs = LogCapture::start("post-oversized", tracing::Level::INFO);
    let masked = "x".repeat(600_000);
    let body = format!(
        r#"{{"action":"allow","category":"benign","response_masked_data":{{"data":"{masked}"}}}}"#
    );
    let server = support::serve(StatusCode::OK, &body).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(TWO_ANSWERS);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let error = post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect_err("failed");
    assert_eq!(error.status, 500);
    assert_eq!(
        error.detail.error.message,
        "Security scan failed - response blocked for safety"
    );
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsScanFailed);
    assert_eq!(error.detail.error.category, "api_error");
    let written = logs.finish();
    assert!(written.contains("masked output would exceed 1048576 bytes"));
}

#[tokio::test]
async fn a_missing_call_id_is_synthesized_for_the_post_marker() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data("{}");
    let mut model_response = response(ANSWER);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("allowed");
    let synthesized = "00000000-0000-0000-0000-000000000007";
    assert_eq!(
        request_data.get("litellm_call_id"),
        Some(&GuardrailValue::Text(synthesized.to_owned()))
    );
    assert_eq!(
        support::bucket_field(&request_data, &format!("_panw_post_scanned_{synthesized}")),
        Some(&GuardrailValue::Boolean(true))
    );
    let body = support::only_scan_body(&server).await;
    assert_eq!(text_of(field(&body, "tr_id")), synthesized);
}

// The extraction view turns a non-object part into nothing to scan; asserted through the rich fixture.
#[tokio::test]
async fn a_non_object_content_part_is_not_scanned() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response =
        response(r#"{"choices":[{"message":{"content":[{"type":"text","text":"only"},7]}}]}"#);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("allowed");
    let body = support::only_scan_body(&server).await;
    assert_eq!(support::scanned_text(&body, "response"), "only");
}

#[tokio::test]
async fn a_suppressed_guardrail_leaves_the_response_untouched() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let original = request_data.clone();
    let mut model_response = response(ANSWER);
    let selection = FakeSelection { events: vec![] };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        &guardrail,
        &mut request_data,
        CallResponse::Model(&mut model_response),
        &mut audit,
        &hook_context,
    )
    .await
    .expect("pass through");
    assert_eq!(request_data, original);
    let message = support::message_at(&model_response, 0);
    assert_eq!(support::text_content(message), "answer");
    assert!(server.body_texts().await.is_empty());
}

#[tokio::test]
async fn a_second_post_call_is_deduplicated() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let mut model_response = response(ANSWER);
    support::run_post_call(&guardrail, &mut request_data, &mut model_response)
        .await
        .expect("first");
    let mut second_response = response(ANSWER);
    support::run_post_call(&guardrail, &mut request_data, &mut second_response)
        .await
        .expect("second");
    assert_eq!(server.body_texts().await.len(), 1);
}
