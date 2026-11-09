use opentelemetry::trace::{
    Link, SpanContext, SpanKind, TraceContextExt as _, TraceId, TraceState,
};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::{SamplingDecision, SamplingResult, ShouldSample};
use sha2::{Digest as _, Sha256};

// litellm's TraceIdHashSampler: a UUID's variant bits pin the low word, so a low-word ratio drops up to 0.5
#[derive(Clone, Copy, Debug)]
pub struct TraceIdHashSampler {
    pub rate: f64,
}

impl ShouldSample for TraceIdHashSampler {
    fn should_sample(
        &self,
        parent_context: Option<&Context>,
        trace_id: TraceId,
        _name: &str,
        _span_kind: &SpanKind,
        attributes: &[KeyValue],
        _links: &[Link],
    ) -> SamplingResult {
        let digest = Sha256::digest(trace_id.to_bytes());
        let prefix = digest.first_chunk::<8>().copied().unwrap_or([0; 8]);
        let sampled = u128::from(u64::from_be_bytes(prefix)) < sampling_threshold(self.rate);
        let trace_state = parent_context
            .map(|context| context.span().span_context().clone())
            .filter(SpanContext::is_valid)
            .map_or(TraceState::NONE, |span_context| {
                span_context.trace_state().clone()
            });
        SamplingResult {
            decision: if sampled {
                SamplingDecision::RecordAndSample
            } else {
                SamplingDecision::Drop
            },
            attributes: if sampled {
                attributes.to_vec()
            } else {
                Vec::new()
            },
            trace_state,
        }
    }
}

// Python's round(rate * 2**64) with ties to even, read from the float's bits so the comparison stays exact
fn sampling_threshold(rate: f64) -> u128 {
    if rate.is_nan() || rate < 0.0_f64 {
        return 0;
    }
    if rate >= 1.0_f64 {
        return u128::MAX;
    }
    let bits = rate.to_bits();
    let exponent = (bits >> 52_u32) & 0x7ff_u64;
    if exponent == 0 {
        return 0;
    }
    let mantissa = u128::from((bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000);
    if exponent >= 1011_u64 {
        let shift = u32::try_from(exponent.saturating_sub(1011_u64)).unwrap_or_default();
        return mantissa.checked_shl(shift).unwrap_or_default();
    }
    let shift_bits = 1011_u64.saturating_sub(exponent);
    if shift_bits >= 64_u64 {
        return 0;
    }
    let shift = u32::try_from(shift_bits).unwrap_or_default();
    let quotient = mantissa.checked_shr(shift).unwrap_or_default();
    let remainder = mantissa & ((1_u128 << shift).saturating_sub(1_u128));
    let half = 1_u128 << shift.saturating_sub(1_u32);
    if remainder > half || (remainder == half && (quotient & 1_u128) == 1_u128) {
        quotient.saturating_add(1_u128)
    } else {
        quotient
    }
}

#[path = "sampler/tests.rs"]
mod tests;
