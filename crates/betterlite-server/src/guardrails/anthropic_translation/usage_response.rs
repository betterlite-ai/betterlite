#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_types,
        reason = "the tests compare provider JSON with serde_json values and plain number literals"
    )
)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_anthropic::usage::error::UsageError;
use betterlite_anthropic::usage::{ReasoningTokenCounter, calculate_usage};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::decisions::Json;
use betterlite_streaming::passthrough_reconstruction::converter::UsageCalculator;
use betterlite_streaming::passthrough_reconstruction::events::RawSseChunk;
use betterlite_streaming::passthrough_reconstruction::usage::{
    UsageOnlyRequest, build_usage_only_response,
};
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use thiserror::Error;
use uuid::Uuid;

use crate::guardrails::anthropic_translation::sse_events::SseChunk;

mod tests;

// LiteLLM reads this usage object with no reasoning content, so the counter is never asked for a count.
pub(super) struct ZeroReasoningCounter;

impl ReasoningTokenCounter for ZeroReasoningCounter {
    type Failure = Infallible;

    #[cfg_attr(test, mutants::skip)]
    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

#[derive(Debug, Error)]
pub enum UsageBridgeError {
    #[error(transparent)]
    Usage(#[from] UsageError<Infallible>),
    #[error(transparent)]
    Convert(#[from] serde_json::Error),
}

// The builder asks for an OpenAI-shaped usage record; LiteLLM's Anthropic usage is mapped through JSON.
struct AnthropicUsageCalculator;

impl UsageCalculator for AnthropicUsageCalculator {
    type Failure = UsageBridgeError;

    fn calculate_usage(
        &self,
        usage_object: &Json,
        speed: Option<&str>,
    ) -> Result<Usage, Self::Failure> {
        let usage = calculate_usage(usage_object, None, None, speed, &ZeroReasoningCounter)?;
        let value = serde_json::to_value(&usage)?;
        Ok(serde_json::from_value(value)?)
    }
}

pub(super) fn unix_seconds() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    i64::try_from(seconds).unwrap_or(i64::MAX)
}

fn model_of(request_data: Option<&BTreeMap<String, GuardrailValue>>) -> &str {
    match request_data.and_then(|data| data.get("model")) {
        Some(GuardrailValue::Text(text)) => text,
        Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        )
        | None => "",
    }
}

// LiteLLM's _build_streaming_usage_response: the usage-only response a blocked stream reports instead.
#[must_use]
pub fn build_streaming_usage_response(
    responses_so_far: &[SseChunk],
    request_data: Option<&BTreeMap<String, GuardrailValue>>,
) -> Option<GuardrailValue> {
    let chunks: Vec<RawSseChunk> = responses_so_far
        .iter()
        .filter_map(|chunk| match chunk {
            SseChunk::Bytes(bytes) => Some(RawSseChunk::Bytes(bytes.clone())),
            SseChunk::Event(_) | SseChunk::Other => None,
        })
        .collect();
    if chunks.is_empty() {
        return None;
    }
    let calculator = AnthropicUsageCalculator;
    let request = UsageOnlyRequest {
        chunks: &chunks,
        model: model_of(request_data),
        speed: None,
        calculator: &calculator,
        now: unix_seconds(),
        next_id: Uuid::new_v4,
    };
    let response = build_usage_only_response(&request).ok()??;
    let value = serde_json::to_value(&response).ok()?;
    serde_json::from_value(value).ok()
}
