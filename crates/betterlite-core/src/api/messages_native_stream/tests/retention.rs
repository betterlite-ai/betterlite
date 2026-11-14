#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_logging::passthrough_stream_logging::types::EndpointType;

use super::super::registry::StreamRegistry;
use super::super::{RelayLimits, RetainedOutputLimit, RetentionBudget, error_sse_event};
use super::support::{Frame, Harness, bytes, pump_of, read};
use crate::api::api_boundaries::LimitErrorFields;
use crate::api::messages_stream_logging::SseChunk;

const LIMIT_MESSAGE: &str = "Messages stream exceeded its retained-output limit";
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
        max_chunks: 65_536,
        bytes: 0,
        chunks: 0,
    }
}

#[tokio::test]
async fn relay_retained_bytes_are_bounded() {
    the_budget_accepts_the_limit_and_rejects_the_first_excess();
    a_detached_overflow_records_one_partial_failure().await;
    a_started_stream_ends_with_the_api_error_event().await;
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
    let pump = pump_of(&relay);
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
    let failures = harness.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].failure.kind, "BadGatewayError");
    assert_eq!(failures[0].failure.message, LIMIT_MESSAGE);
    assert_eq!(failures[0].endpoint, EndpointType::Anthropic);
    assert_eq!(failures[0].raw_bytes.len(), 16);
    assert_eq!(registry.sizes().live_pumps, 0);
    assert_eq!(registry.sizes().detached_drains, 0);
}

async fn a_started_stream_ends_with_the_api_error_event() {
    let registry = Arc::new(StreamRegistry::default());
    let (harness, mut relay) = Harness::start(limits(), Arc::clone(&registry));
    for _ in 0..16_u8 {
        harness.script.release(SseChunk::Bytes(bytes(LIMIT))).await;
        assert!(matches!(read(&mut relay).await, Frame::Chunk(_)));
    }
    harness.script.release(SseChunk::Bytes(bytes(LIMIT))).await;
    let event = String::from_utf8(error_sse_event(LIMIT_MESSAGE)).expect("the event is UTF-8");
    assert_eq!(read(&mut relay).await, Frame::Chunk(event.clone()));
    assert!(event.starts_with("event: error\ndata: "));
    assert!(event.contains("\"type\":\"api_error\""));
    assert!(event.contains(LIMIT_MESSAGE));
    assert!(!event.contains("message_stop"));
    assert!(!event.contains("\"param\""));
    assert!(!event.contains("\"code\""));
    let pump = pump_of(&relay);
    pump.wait().await;
    assert_eq!(read(&mut relay).await, Frame::End);
    assert!(harness.closed.load(Ordering::SeqCst));
    assert_eq!(harness.billed(), 0);
    assert_eq!(harness.failures().len(), 1);
    assert_eq!(registry.sizes().live_pumps, 0);
}
