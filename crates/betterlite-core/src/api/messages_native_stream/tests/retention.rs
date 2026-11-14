#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_logging::passthrough_stream_logging::types::EndpointType;

use super::super::RelayLimits;
use super::super::registry::StreamRegistry;
use super::super::retention::{RetainedOutputLimit, RetentionBudget};
use super::support::{Frame, Harness, bytes, pump_of, read, release_and_read};
use crate::api::api_boundaries::{ApiLimits, LimitErrorFields};
use crate::api::messages_stream_logging::SseChunk;

const LIMIT_MESSAGE: &str = "Messages stream exceeded its retained-output limit";
const LIMIT_EVENT: &str = "event: error\ndata: {\"type\": \"error\", \"error\": {\"type\": \"api_error\", \"message\": \"Messages stream exceeded its retained-output limit\"}}\n\n";
const LIMIT: usize = 1024 * 1024;

fn limits() -> RelayLimits {
    RelayLimits {
        queue_maxsize: 8,
        max_detached_stream_drains: 100,
    }
}

fn budget() -> RetentionBudget {
    RetentionBudget {
        max_bytes: 16 * LIMIT,
        max_chunks: 0x0001_0000,
        bytes: 0,
        chunks: 0,
    }
}

#[tokio::test]
async fn relay_retained_bytes_are_bounded() {
    the_budget_accepts_the_limit_and_rejects_the_first_excess();
    a_detached_overflow_records_one_partial_failure().await;
    a_started_stream_ends_with_the_api_error_event().await;
    a_started_stream_stops_at_the_chunk_limit().await;
}

fn the_budget_accepts_the_limit_and_rejects_the_first_excess() {
    let mut byte_budget = budget();
    assert_eq!(byte_budget.accept(16 * LIMIT), Ok(()));
    assert_eq!(byte_budget.accept(1), Err(RetainedOutputLimit));
    assert_eq!(byte_budget.accept(usize::MAX), Err(RetainedOutputLimit));
    let mut chunk_budget = budget();
    for _ in 0..chunk_budget.max_chunks {
        assert_eq!(chunk_budget.accept(1), Ok(()));
    }
    assert_eq!(chunk_budget.accept(1), Err(RetainedOutputLimit));
    assert_eq!(
        RetainedOutputLimit::FIELDS,
        LimitErrorFields {
            status: 502,
            error_type: "bad_gateway",
            param: "stream",
            code: "stream_buffer_too_large",
        }
    );
}

async fn a_detached_overflow_records_one_partial_failure() {
    let registry = Arc::new(StreamRegistry::default());
    let (harness, mut relay) = Harness::start(limits(), Arc::clone(&registry));
    harness.script.release(SseChunk::Bytes(bytes(LIMIT))).await;
    assert!(matches!(read(&mut relay).await, Frame::Chunk(_)));
    let pump = pump_of(&harness);
    drop(relay);
    for _ in 0..15_u8 {
        harness.script.release(SseChunk::Bytes(bytes(LIMIT))).await;
    }
    // the seventeenth chunk is the first excess, and it is never appended to the billing bytes
    harness.script.release(SseChunk::Bytes(bytes(LIMIT))).await;
    pump.wait().await;
    harness.worker.flush().await;
    assert!(harness.closed.load(Ordering::SeqCst));
    assert_eq!(harness.billed(), 0);
    assert_the_overflow_failure(&harness);
    assert_eq!(registry.sizes().live_pumps, 0);
    assert_eq!(registry.sizes().detached_drains, 0);
}

fn assert_the_overflow_failure(harness: &Harness) {
    let failures = harness.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].failure.kind, "BadGatewayError");
    assert_eq!(failures[0].failure.message, LIMIT_MESSAGE);
    assert_eq!(failures[0].endpoint, EndpointType::Anthropic);
    assert_eq!(failures[0].raw_bytes.len(), 16);
}

async fn a_started_stream_ends_with_the_api_error_event() {
    let registry = Arc::new(StreamRegistry::default());
    let (harness, mut relay) = Harness::start(limits(), Arc::clone(&registry));
    for _ in 0..16_u8 {
        let chunk = SseChunk::Bytes(bytes(LIMIT));
        assert!(matches!(
            release_and_read(&harness, &mut relay, chunk).await,
            Frame::Chunk(_)
        ));
    }
    harness.script.release(SseChunk::Bytes(bytes(LIMIT))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk(LIMIT_EVENT.to_owned()));
    let pump = pump_of(&harness);
    pump.wait().await;
    assert_eq!(read(&mut relay).await, Frame::End);
    assert!(harness.closed.load(Ordering::SeqCst));
    assert_eq!(harness.billed(), 0);
    assert_eq!(harness.failures().len(), 1);
    assert_eq!(registry.sizes().live_pumps, 0);
}

async fn a_started_stream_stops_at_the_chunk_limit() {
    let registry = Arc::new(StreamRegistry::default());
    let boundaries = ApiLimits {
        max_json_bytes: LIMIT,
        max_event_bytes: LIMIT,
        max_items: 3,
    };
    let (harness, mut relay) = Harness::start_with(limits(), Arc::clone(&registry), boundaries);
    for _ in 0..3_u8 {
        let chunk = SseChunk::Bytes(bytes(1));
        assert_eq!(
            release_and_read(&harness, &mut relay, chunk).await,
            Frame::Chunk("x".to_owned())
        );
    }
    // the fourth chunk is the first excess: it never reaches the client and never bills
    harness.script.release(SseChunk::Bytes(bytes(1))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk(LIMIT_EVENT.to_owned()));
    let pump = pump_of(&harness);
    pump.wait().await;
    assert_eq!(read(&mut relay).await, Frame::End);
    harness.worker.flush().await;
    assert!(harness.closed.load(Ordering::SeqCst));
    assert_eq!(harness.billed(), 0);
    assert_the_item_limit_failure(&harness);
    assert_eq!(registry.sizes().live_pumps, 0);
}

fn assert_the_item_limit_failure(harness: &Harness) {
    let failures = harness.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].failure.kind, "BadGatewayError");
    assert_eq!(failures[0].failure.message, LIMIT_MESSAGE);
    assert_eq!(failures[0].raw_bytes.len(), 3);
}
