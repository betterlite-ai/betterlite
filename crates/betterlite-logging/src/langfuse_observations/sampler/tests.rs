#![cfg(test)]

use std::str::FromStr as _;

use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::{SamplingDecision, ShouldSample as _};
use rstest::rstest;

use super::{TraceIdHashSampler, sampling_threshold};

#[derive(Clone, Copy, Debug)]
enum Check {
    ZeroRateDropsEverything,
    FullRateSamplesEverything,
    HalfRateKeepsLowDigests,
    HalfRateDropsHighDigests,
    QuarterRateKeepsLowDigests,
    QuarterRateDropsHigherDigests,
    TinyRateKeepsOnlyTheLowestDigests,
    SampledDecisionKeepsAttributes,
    DroppedDecisionDropsAttributes,
    ParentTraceStateTravels,
    InvalidParentContributesNothing,
}

fn trace(text: &str) -> TraceId {
    TraceId::from_hex(text).unwrap()
}

fn decision(rate: f64, trace_id: TraceId, attributes: &[KeyValue]) -> SamplingDecision {
    TraceIdHashSampler { rate }
        .should_sample(None, trace_id, "span", &SpanKind::Internal, attributes, &[])
        .decision
}

fn parent_context(trace_state: TraceState) -> Context {
    let span_context = SpanContext::new(
        trace("0102030405060708090a0b0c0d0e0f10"),
        SpanId::from_hex("0102030405060708").unwrap(),
        TraceFlags::SAMPLED,
        false,
        trace_state,
    );
    Context::new().with_remote_span_context(span_context)
}

#[rstest]
#[case::zero_rate_drops_everything(Check::ZeroRateDropsEverything)]
#[case::full_rate_samples_everything(Check::FullRateSamplesEverything)]
#[case::half_rate_keeps_low_digests(Check::HalfRateKeepsLowDigests)]
#[case::half_rate_drops_high_digests(Check::HalfRateDropsHighDigests)]
#[case::quarter_rate_keeps_low_digests(Check::QuarterRateKeepsLowDigests)]
#[case::quarter_rate_drops_higher_digests(Check::QuarterRateDropsHigherDigests)]
#[case::tiny_rate_keeps_only_the_lowest_digests(Check::TinyRateKeepsOnlyTheLowestDigests)]
#[case::sampled_decision_keeps_attributes(Check::SampledDecisionKeepsAttributes)]
#[case::dropped_decision_drops_attributes(Check::DroppedDecisionDropsAttributes)]
#[case::parent_trace_state_travels(Check::ParentTraceStateTravels)]
#[case::invalid_parent_contributes_nothing(Check::InvalidParentContributesNothing)]
fn observation_attributes_tests_behavior(#[case] check: Check) {
    match check {
        Check::ZeroRateDropsEverything => {
            assert_eq!(
                decision(0.0, trace("00000000000000000000000000000000"), &[]),
                SamplingDecision::Drop
            );
        }
        Check::FullRateSamplesEverything => {
            for text in [
                "00000000000000000000000000000000",
                "ffffffffffffffffffffffffffffffff",
                "00000000000000000000000000000003",
            ] {
                assert_eq!(
                    decision(1.0, trace(text), &[]),
                    SamplingDecision::RecordAndSample
                );
            }
        }
        Check::HalfRateKeepsLowDigests => {
            assert_eq!(
                decision(0.5, trace("00000000000000000000000000000000"), &[]),
                SamplingDecision::RecordAndSample
            );
        }
        Check::HalfRateDropsHighDigests => {
            assert_eq!(
                decision(0.5, trace("00000000000000000000000000000003"), &[]),
                SamplingDecision::Drop
            );
        }
        Check::QuarterRateKeepsLowDigests => {
            assert_eq!(
                decision(0.25, trace("00000000000000000000000000000000"), &[]),
                SamplingDecision::RecordAndSample
            );
        }
        Check::QuarterRateDropsHigherDigests => {
            assert_eq!(
                decision(0.25, trace("00000000000000000000000000000001"), &[]),
                SamplingDecision::Drop
            );
        }
        Check::TinyRateKeepsOnlyTheLowestDigests => {
            assert_eq!(sampling_threshold(1e-4), 1_844_674_407_370_955);
            assert_eq!(sampling_threshold(2.0_f64.powi(-65)), 0);
            assert_eq!(sampling_threshold(f64::from_bits(0x3bf8_0000_0000_0000)), 2);
            assert_eq!(sampling_threshold(f64::from_bits(0x3c04_0000_0000_0000)), 2);
            assert_eq!(
                decision(1e-4, trace("000000000000000000000000000009be"), &[]),
                SamplingDecision::RecordAndSample
            );
            assert_eq!(
                decision(1e-4, trace("00000000000000000000000000000000"), &[]),
                SamplingDecision::Drop
            );
            assert_eq!(
                decision(1e-30, trace("00000000000000000000000000000000"), &[]),
                SamplingDecision::Drop
            );
        }
        Check::SampledDecisionKeepsAttributes => {
            let result = TraceIdHashSampler { rate: 1.0 }.should_sample(
                None,
                trace("00000000000000000000000000000000"),
                "span",
                &SpanKind::Internal,
                &[KeyValue::new("marker", "kept")],
                &[],
            );
            assert_eq!(result.decision, SamplingDecision::RecordAndSample);
            assert_eq!(result.attributes, vec![KeyValue::new("marker", "kept")]);
        }
        Check::DroppedDecisionDropsAttributes => {
            let result = TraceIdHashSampler { rate: 0.0 }.should_sample(
                None,
                trace("00000000000000000000000000000000"),
                "span",
                &SpanKind::Internal,
                &[KeyValue::new("marker", "kept")],
                &[],
            );
            assert_eq!(result.decision, SamplingDecision::Drop);
            assert!(result.attributes.is_empty());
        }
        Check::ParentTraceStateTravels => {
            let trace_state = TraceState::from_str("vendor=value").unwrap();
            let context = parent_context(trace_state.clone());
            let result = TraceIdHashSampler { rate: 1.0 }.should_sample(
                Some(&context),
                trace("00000000000000000000000000000000"),
                "span",
                &SpanKind::Internal,
                &[],
                &[],
            );
            assert_eq!(result.trace_state, trace_state);
        }
        Check::InvalidParentContributesNothing => {
            let result = TraceIdHashSampler { rate: 1.0 }.should_sample(
                Some(&Context::new()),
                trace("00000000000000000000000000000000"),
                "span",
                &SpanKind::Internal,
                &[],
                &[],
            );
            assert_eq!(result.trace_state, TraceState::NONE);
        }
    }
}
