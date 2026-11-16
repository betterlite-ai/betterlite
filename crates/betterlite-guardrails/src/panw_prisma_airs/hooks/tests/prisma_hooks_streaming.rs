use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::panw_prisma_airs::FallbackAction;

use super::support::{
    self, APPLIED, FakeAssembler, FakeSelection, NO_MASKING, context, data, field, guardrail, json,
    names_in, records, scan_entries, settings, text_of,
};
use crate::guardrail_message_scope::StreamItem;
use crate::panw_prisma_airs::audit::GuardrailAudit;
use crate::panw_prisma_airs::hooks::streaming::hook::streaming_iterator_hook;
use crate::panw_prisma_airs::hooks::streaming::{StreamChunk, StreamingResponse, into_items};

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;
const BLOCK: &str = r#"{"action":"block","category":"toxic","scan_id":"scan-2"}"#;
const ASSEMBLED: &str = r#"{"choices":[{"message":{"content":"answer"}}]}"#;

fn model(id: &str) -> StreamChunk {
    StreamChunk::Model(json(&format!(r#"{{"id":"{id}"}}"#)))
}

fn delta(text: &str) -> StreamChunk {
    StreamChunk::Bytes(
        format!(
            "data: {{\"type\":\"content_block_delta\",\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}\n\n"
        )
        .into_bytes(),
    )
}

#[tokio::test]
async fn disabled_passes_the_chunks_through() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, NO_MASKING, FallbackAction::Block));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![model("a"), model("b")];
    let expected = into_items(chunks.clone());
    let fake = FakeAssembler::new(Some(ASSEMBLED));
    let selection = FakeSelection { events: vec![] };
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
    .expect("pass through");
    assert_eq!(items, expected);
    assert_eq!(fake.assembled_chunk_count(), 0);
    assert!(server.body_texts().await.is_empty());
    assert!(request_data.get("litellm_metadata").is_none());
}

#[tokio::test]
async fn bytes_chunks_are_scanned_and_passed_through() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![delta("ab"), delta("cd")];
    let expected = into_items(chunks.clone());
    let fake = FakeAssembler::new(Some(ASSEMBLED));
    let items = support::run_streaming(&guardrail, &mut request_data, chunks, &fake).await;
    assert_eq!(items, expected);
    assert_eq!(fake.assembled_chunk_count(), 0);
    assert_eq!(
        support::bucket_field(&request_data, "_panw_streaming_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert_eq!(names_in(&request_data, APPLIED), vec!["panw".to_owned()]);
    let record = records(&request_data).first().expect("record");
    assert_eq!(text_of(field(record, "guardrail_mode")), "post_call");
    assert_eq!(text_of(field(record, "guardrail_status")), "success");
    let bodies = server.body_texts().await;
    let body = json(bodies.first().expect("one scan"));
    let contents = support::array_of(field(&body, "contents"));
    assert_eq!(
        text_of(field(contents.first().expect("content"), "response")),
        "abcd"
    );
}

#[tokio::test]
async fn event_chunks_are_scanned_and_passed_through() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, NO_MASKING, FallbackAction::Block));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![
        StreamChunk::Event(json(r#"{"type":"response.output_text.delta","delta":"a"}"#)),
        StreamChunk::Event(json(r#"{"type":"response.output_text.delta","delta":"b"}"#)),
    ];
    let expected = into_items(chunks.clone());
    let fake = FakeAssembler::new(Some(ASSEMBLED));
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
    .expect("pass through");
    assert_eq!(items, expected);
    assert_eq!(fake.assembled_chunk_count(), 0);
    let bodies = server.body_texts().await;
    let body = json(bodies.first().expect("one scan"));
    let contents = support::array_of(field(&body, "contents"));
    assert_eq!(
        text_of(field(contents.first().expect("content"), "response")),
        "ab"
    );
}

#[tokio::test]
async fn the_first_chunk_decides_the_route() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, NO_MASKING, FallbackAction::Block));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![delta("raw"), model("late")];
    let expected = into_items(chunks.clone());
    let fake = FakeAssembler::new(Some(ASSEMBLED));
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
    .expect("pass through");
    assert_eq!(items, expected);
    assert_eq!(fake.assembled_chunk_count(), 0);
    let bodies = server.body_texts().await;
    let body = json(bodies.first().expect("one scan"));
    let contents = support::array_of(field(&body, "contents"));
    assert_eq!(
        text_of(field(contents.first().expect("content"), "response")),
        "raw"
    );
}

#[tokio::test]
async fn model_chunks_are_assembled_with_every_chunk() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, NO_MASKING, FallbackAction::Block));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![model("a"), model("b")];
    let expected = into_items(chunks.clone());
    let fake = FakeAssembler::new(Some(ASSEMBLED));
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
    .expect("pass through");
    assert_eq!(items, expected);
    assert_eq!(fake.assembled_chunk_count(), 2);
    assert_eq!(fake.reassembled_count(), 0);
    assert_eq!(names_in(&request_data, APPLIED), vec!["panw".to_owned()]);
    let entries = scan_entries(&request_data);
    assert_eq!(entries.first().expect("scan entry").scan_id, "scan-1");
    let bodies = server.body_texts().await;
    let body = json(bodies.first().expect("one scan"));
    let contents = support::array_of(field(&body, "contents"));
    assert_eq!(
        text_of(field(contents.first().expect("content"), "response")),
        "answer"
    );
}

#[tokio::test]
async fn a_masked_assembly_returns_the_rebuilt_chunks() {
    let server = support::serve(
        StatusCode::OK,
        r#"{"action":"allow","category":"benign","response_masked_data":{"data":"MASKED"}}"#,
    )
    .await;
    let guardrail = guardrail(settings(&server.base, NO_MASKING, FallbackAction::Block));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![model("a")];
    let fake = FakeAssembler::new(Some(ASSEMBLED));
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
    .expect("masked");
    assert_eq!(items, vec![StreamItem::Bytes(b"data: MASKED\n\n".to_vec())]);
    assert_eq!(fake.reassembled_count(), 1);
    assert_eq!(names_in(&request_data, APPLIED), vec!["panw".to_owned()]);
    let record = records(&request_data).first().expect("record");
    assert_eq!(text_of(field(record, "guardrail_status")), "success");
}

#[tokio::test]
async fn a_blocked_assembly_yields_an_error_frame() {
    let server = support::serve(StatusCode::OK, BLOCK).await;
    let guardrail = guardrail(settings(&server.base, NO_MASKING, FallbackAction::Block));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![model("a")];
    let fake = FakeAssembler::new(Some(ASSEMBLED));
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
    .expect("blocked frame");
    assert_eq!(items.len(), 1);
    let frame = support::frame_json(&items[0]);
    let error = field(&frame, "error");
    assert_eq!(field(error, "code"), &GuardrailValue::SignedInteger(400));
    assert_eq!(text_of(field(error, "type")), "guardrail_violation");
    assert_eq!(text_of(field(error, "action")), "block");
    assert_eq!(
        text_of(field(error, "message")),
        "Response blocked by PANW Prisma AI Security policy (Category: toxic)"
    );
}

#[tokio::test]
async fn a_none_assembly_passes_the_chunks() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, NO_MASKING, FallbackAction::Block));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![model("a"), model("b")];
    let expected = into_items(chunks.clone());
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
    .expect("pass through");
    assert_eq!(items, expected);
    assert_eq!(fake.assembled_chunk_count(), 2);
    assert!(server.body_texts().await.is_empty());
    assert_eq!(
        support::bucket_field(&request_data, "_panw_streaming_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[tokio::test]
async fn a_second_streaming_call_is_deduplicated() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = support::blocking(&server.base);
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let fake = FakeAssembler::new(Some(ASSEMBLED));
    let original = vec![model("a")];
    let expected = into_items(original.clone());
    let first = support::run_streaming(&guardrail, &mut request_data, original, &fake).await;
    let again = vec![model("a")];
    let second = support::run_streaming(&guardrail, &mut request_data, again, &fake).await;
    assert_eq!(first, expected);
    assert_eq!(second, expected);
    assert_eq!(fake.assembled_chunk_count(), 1);
    assert_eq!(server.body_texts().await.len(), 1);
}

#[tokio::test]
async fn an_empty_stream_yields_nothing() {
    let server = support::serve(StatusCode::OK, ALLOW).await;
    let guardrail = guardrail(settings(&server.base, NO_MASKING, FallbackAction::Block));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let fake = FakeAssembler::new(Some(ASSEMBLED));
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
            chunks: vec![],
            assembler: &fake,
        },
        &mut audit,
        &hook_context,
    )
    .await
    .expect("empty");
    assert!(items.is_empty());
    assert!(server.body_texts().await.is_empty());
    assert_eq!(fake.assembled_chunk_count(), 0);
    assert_eq!(
        support::bucket_field(&request_data, "_panw_streaming_scanned_call-1"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[tokio::test]
async fn a_transient_fallback_yields_the_originals() {
    let server = support::serve(StatusCode::TOO_MANY_REQUESTS, "{}").await;
    let guardrail = guardrail(settings(&server.base, NO_MASKING, FallbackAction::Allow));
    let mut request_data = data(r#"{"litellm_call_id":"call-1"}"#);
    let chunks = vec![model("a")];
    let expected = into_items(chunks.clone());
    let fake = FakeAssembler::new(Some(ASSEMBLED));
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
    .expect("unscanned");
    assert_eq!(items, expected);
    assert_eq!(fake.reassembled_count(), 0);
    assert_eq!(
        names_in(&request_data, APPLIED),
        vec!["panw:unscanned".to_owned()]
    );
    let record = records(&request_data).first().expect("record");
    assert_eq!(
        text_of(field(record, "guardrail_status")),
        "guardrail_failed_to_respond"
    );
}
