#![cfg(test)]

use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry_sdk::trace::IdGenerator as _;
use rstest::rstest;

use super::{RequestedIdGenerator, RequestedIds, requested_ids};

#[derive(Clone, Copy, Debug)]
enum Check {
    RequestedIdsAreHandedOut,
    AbsentRequestFallsBackToRandom,
    ScopeClearsOnDrop,
    NestedScopesRestoreTheOuterIds,
    GetterReflectsTheScope,
    SpanOnlyRequestLeavesTraceRandom,
}

fn trace(text: &str) -> TraceId {
    TraceId::from_hex(text).unwrap()
}

fn span(text: &str) -> SpanId {
    SpanId::from_hex(text).unwrap()
}

#[rstest]
#[case::requested_ids_are_handed_out(Check::RequestedIdsAreHandedOut)]
#[case::absent_request_falls_back_to_random(Check::AbsentRequestFallsBackToRandom)]
#[case::scope_clears_on_drop(Check::ScopeClearsOnDrop)]
#[case::nested_scopes_restore_the_outer_ids(Check::NestedScopesRestoreTheOuterIds)]
#[case::getter_reflects_the_scope(Check::GetterReflectsTheScope)]
#[case::span_only_request_leaves_trace_random(Check::SpanOnlyRequestLeavesTraceRandom)]
fn observation_attributes_tests_behavior(#[case] check: Check) {
    let generator = RequestedIdGenerator::default();
    match check {
        Check::RequestedIdsAreHandedOut => {
            let _scope = RequestedIds::enter(
                Some(trace("0102030405060708090a0b0c0d0e0f10")),
                Some(span("0102030405060708")),
            );
            assert_eq!(
                generator.new_trace_id(),
                trace("0102030405060708090a0b0c0d0e0f10")
            );
            assert_eq!(generator.new_span_id(), span("0102030405060708"));
        }
        Check::AbsentRequestFallsBackToRandom => {
            let fallback_trace = generator.new_trace_id();
            let fallback_span = generator.new_span_id();
            assert_ne!(fallback_trace, TraceId::INVALID);
            assert_ne!(fallback_span, SpanId::INVALID);
            assert_eq!(fallback_trace.to_string().len(), 32);
            assert_eq!(fallback_span.to_string().len(), 16);
        }
        Check::ScopeClearsOnDrop => {
            {
                let _scope = RequestedIds::enter(
                    Some(trace("0102030405060708090a0b0c0d0e0f10")),
                    Some(span("0102030405060708")),
                );
            }
            assert_ne!(generator.new_trace_id(), TraceId::INVALID);
            assert_eq!(requested_ids(), (None, None));
        }
        Check::NestedScopesRestoreTheOuterIds => {
            let outer = RequestedIds::enter(
                Some(trace("0102030405060708090a0b0c0d0e0f10")),
                Some(span("0102030405060708")),
            );
            let inner = RequestedIds::enter(
                Some(trace("1112131415161718191a1b1c1d1e1f20")),
                Some(span("1112131415161718")),
            );
            assert_eq!(
                generator.new_trace_id(),
                trace("1112131415161718191a1b1c1d1e1f20")
            );
            drop(inner);
            assert_eq!(
                generator.new_trace_id(),
                trace("0102030405060708090a0b0c0d0e0f10")
            );
            assert_eq!(generator.new_span_id(), span("0102030405060708"));
            drop(outer);
            assert_eq!(requested_ids(), (None, None));
        }
        Check::GetterReflectsTheScope => {
            let _scope = RequestedIds::enter(
                Some(trace("0102030405060708090a0b0c0d0e0f10")),
                Some(span("0102030405060708")),
            );
            assert_eq!(
                requested_ids(),
                (
                    Some(trace("0102030405060708090a0b0c0d0e0f10")),
                    Some(span("0102030405060708"))
                )
            );
        }
        Check::SpanOnlyRequestLeavesTraceRandom => {
            let _scope = RequestedIds::enter(None, Some(span("0102030405060708")));
            assert_ne!(generator.new_trace_id(), TraceId::INVALID);
            assert_eq!(generator.new_span_id(), span("0102030405060708"));
        }
    }
}
