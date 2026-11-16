use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::panw_prisma_airs::FallbackAction;
use indexmap::IndexMap;
use serde_json::value::RawValue;

use super::support::{
    self, FakeAssembler, FakeSelection, LogCapture, context, data, field, guardrail, no_masking,
    object_of, settings, text_of,
};
use crate::guardrail_message_scope::StreamItem;
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::error_detail::{
    ErrorBody, ErrorCode, ErrorDetail, ErrorType, GuardrailHttpError,
};
use crate::panw_prisma_airs::hooks::error::{HookDataError, internal};
use crate::panw_prisma_airs::hooks::streaming::hook::streaming_iterator_hook;
use crate::panw_prisma_airs::hooks::streaming::{
    StreamAssemblyError, StreamChunk, StreamError, StreamingResponse, frames,
};
use crate::panw_prisma_airs::verdict::Action;

const BLOCK: &str = r#"{"action":"block","category":"toxic","scan_id":"scan-2"}"#;
const ASSEMBLED: &str = r#"{"choices":[{"message":{"content":"answer"}}]}"#;

fn http_error_fields() -> IndexMap<String, Box<RawValue>> {
    let mut scan_fields = IndexMap::new();
    scan_fields.insert(
        "scan_id".to_owned(),
        RawValue::from_string("\"scan-9\"".to_owned()).expect("raw value"),
    );
    scan_fields
}

#[test]
fn an_http_frame_uses_the_status_as_its_code() {
    let error = StreamError::Http(GuardrailHttpError {
        status: 403,
        detail: Box::new(ErrorDetail {
            error: ErrorBody {
                action: Some(Action::Block),
                scan_fields: http_error_fields(),
                message: "blocked".to_owned(),
                kind: ErrorType::GuardrailViolation,
                code: ErrorCode::PanwPrismaAirsBlocked,
                guardrail: "panw".to_owned(),
                category: "cat".to_owned(),
            },
        }),
    });
    let item = frames::error_item("panw", error);
    let StreamItem::Bytes(bytes) = &item else {
        panic!("frame is not bytes");
    };
    assert!(bytes.starts_with(b"data: "));
    assert!(bytes.ends_with(b"\n\n"));
    let frame = support::frame_json(&item);
    let body = field(&frame, "error");
    assert_eq!(field(body, "code"), &GuardrailValue::SignedInteger(403));
    assert_eq!(text_of(field(body, "type")), "guardrail_violation");
    assert_eq!(text_of(field(body, "action")), "block");
    assert_eq!(text_of(field(body, "guardrail")), "panw");
    assert_eq!(text_of(field(body, "category")), "cat");
    assert_eq!(text_of(field(body, "message")), "blocked");
    assert_eq!(text_of(field(body, "scan_id")), "scan-9");
}

#[test]
fn an_internal_500_frame_carries_no_action() {
    let guardrail = guardrail(settings(
        "http://127.0.0.1:1",
        no_masking(),
        FallbackAction::Block,
    ));
    let http = internal(
        &HookDataError::NotList { field: "messages" },
        Direction::Response,
        &guardrail,
    );
    let item = frames::error_item("panw", StreamError::Http(http));
    let frame = support::frame_json(&item);
    let body = field(&frame, "error");
    assert_eq!(field(body, "code"), &GuardrailValue::SignedInteger(500));
    assert_eq!(
        text_of(field(body, "message")),
        "Security scan failed - response blocked for safety"
    );
    assert_eq!(text_of(field(body, "type")), "guardrail_scan_error");
    assert_eq!(text_of(field(body, "category")), "api_error");
    assert!(object_of(body).get("action").is_none());
}

#[test]
fn a_generic_frame_logs_the_error() {
    let logs = LogCapture::start("frame-generic", tracing::Level::INFO);
    let item = frames::error_item(
        "panw",
        StreamError::Assembly {
            source: StreamAssemblyError,
        },
    );
    let frame = support::frame_json(&item);
    let body = field(&frame, "error");
    assert_eq!(
        text_of(field(body, "message")),
        "Security scan failed - streaming response blocked for safety"
    );
    assert_eq!(text_of(field(body, "type")), "guardrail_scan_error");
    assert_eq!(field(body, "code"), &GuardrailValue::SignedInteger(500));
    assert_eq!(text_of(field(body, "guardrail")), "panw");
    assert!(object_of(body).get("action").is_none());
    let written = logs.finish();
    assert!(written.contains(
        "PANW Prisma AIRS streaming error: the streamed response could not be assembled or rebuilt"
    ));
}

#[tokio::test]
async fn a_blocked_raw_stream_yields_a_frame() {
    let server = support::serve(StatusCode::OK, BLOCK).await;
    let guardrail = guardrail(settings(&server.base, no_masking(), FallbackAction::Block));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![StreamChunk::Bytes(
        b"data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"secret\"}}\n\n"
            .to_vec(),
    )];
    let fake = FakeAssembler::new(None);
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    let items = streaming_iterator_hook(
        &guardrail,
        &mut request_data,
        StreamingResponse {
            chunks,
            assembler: &fake,
        },
        &mut audit,
        &hook_context,
    )
    .await
    .expect("frame");
    assert_eq!(items.len(), 1);
    let frame = support::frame_json(&items[0]);
    let body = field(&frame, "error");
    assert_eq!(field(body, "code"), &GuardrailValue::SignedInteger(400));
    assert_eq!(text_of(field(body, "type")), "guardrail_violation");
    assert_eq!(text_of(field(body, "action")), "block");
    assert_eq!(
        text_of(field(body, "message")),
        "Response blocked by PANW Prisma AI Security policy (Category: toxic)"
    );
}

#[tokio::test]
async fn a_masking_failure_yields_the_generic_frame() {
    let logs = LogCapture::start("frame-masking", tracing::Level::INFO);
    let masked = "x".repeat(1024 * 1024 + 1);
    let body = format!(
        r#"{{"action":"allow","category":"benign","response_masked_data":{{"data":"{masked}"}}}}"#
    );
    let server = support::serve(StatusCode::OK, &body).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![StreamChunk::Model(support::json(r#"{"id":"chunk-1"}"#))];
    let fake = FakeAssembler::new(Some(ASSEMBLED));
    let items = support::run_streaming(&guardrail, &mut request_data, chunks, &fake).await;
    assert_eq!(items.len(), 1);
    let frame = support::frame_json(&items[0]);
    let error = field(&frame, "error");
    assert_eq!(field(error, "code"), &GuardrailValue::SignedInteger(500));
    assert_eq!(text_of(field(error, "type")), "guardrail_scan_error");
    assert_eq!(
        text_of(field(error, "message")),
        "Security scan failed - streaming response blocked for safety"
    );
    let written = logs.finish();
    assert!(written.contains("PANW Prisma AIRS streaming error:"));
    assert!(written.contains("masked output would exceed 1048576 bytes"));
}

#[tokio::test]
async fn a_permanent_scan_failure_yields_the_http_frame() {
    let server = support::serve(StatusCode::BAD_REQUEST, "{}").await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![StreamChunk::Model(support::json(r#"{"id":"chunk-1"}"#))];
    let fake = FakeAssembler::new(Some(ASSEMBLED));
    let items = support::run_streaming(&guardrail, &mut request_data, chunks, &fake).await;
    assert_eq!(items.len(), 1);
    let frame = support::frame_json(&items[0]);
    let error = field(&frame, "error");
    assert_eq!(field(error, "code"), &GuardrailValue::SignedInteger(500));
    assert_eq!(text_of(field(error, "type")), "guardrail_scan_error");
    assert_eq!(text_of(field(error, "category")), "http_400_error");
    assert_eq!(text_of(field(error, "guardrail")), "panw");
}
