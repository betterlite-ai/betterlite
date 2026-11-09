use chrono::{DateTime, Utc};
use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry_sdk::trace::IdGenerator;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

const TRACE_ID_LENGTH: usize = 32;
const OBSERVATION_ID_LENGTH: usize = 16;
const NANOS_PER_SECOND: u32 = 1_000_000_000;

// litellm.integrations.langfuse.langfuse_sdk.to_unix_nanos; guardrail entries carry seconds, model calls a datetime
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Timestamp {
    DateTime(DateTime<Utc>),
    Seconds(Decimal),
}

#[must_use]
pub fn to_unix_nanos(value: Option<Timestamp>) -> Option<i64> {
    match value? {
        Timestamp::DateTime(datetime) => datetime.timestamp_nanos_opt(),
        Timestamp::Seconds(seconds) => seconds
            .checked_mul(Decimal::from(NANOS_PER_SECOND))?
            .to_i64(),
    }
}

// litellm.integrations.langfuse.langfuse_sdk.resolve_trace_id; a caller's id becomes 32 lowercase hex characters
#[must_use]
pub fn resolve_trace_id(trace_id: Option<&str>, id_generator: &dyn IdGenerator) -> TraceId {
    let serialized = trace_id.unwrap_or_default();
    let normalized = serialized.to_lowercase().replace('-', "");
    if is_hex_id(&normalized, TRACE_ID_LENGTH)
        && let Ok(resolved) = TraceId::from_hex(&normalized)
    {
        return resolved;
    }
    if serialized.is_empty() {
        return id_generator.new_trace_id();
    }
    TraceId::from_bytes(hash_prefix::<16>(serialized))
}

// litellm.integrations.langfuse.langfuse_sdk.resolve_observation_id; v4 parent ids are 16 lowercase hex characters
#[must_use]
pub fn resolve_observation_id(observation_id: Option<&str>) -> Option<SpanId> {
    let serialized = observation_id.unwrap_or_default();
    let normalized = serialized.to_lowercase().replace('-', "");
    if is_hex_id(&normalized, OBSERVATION_ID_LENGTH) {
        return SpanId::from_hex(&normalized).ok();
    }
    if serialized.is_empty() {
        return None;
    }
    Some(SpanId::from_bytes(hash_prefix::<8>(serialized)))
}

// the lookahead rejects an all-zero id, which OTel reads as no id at all
fn is_hex_id(candidate: &str, length: usize) -> bool {
    candidate.len() == length
        && candidate.bytes().all(|byte| byte.is_ascii_hexdigit())
        && candidate.bytes().any(|byte| byte != b'0')
}

fn hash_prefix<const LENGTH: usize>(serialized: &str) -> [u8; LENGTH] {
    let digest = Sha256::digest(serialized.as_bytes());
    digest
        .first_chunk::<LENGTH>()
        .copied()
        .unwrap_or([0; LENGTH])
}

#[path = "identity/tests.rs"]
mod tests;
