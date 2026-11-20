use std::time::Duration;

use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_bedrock_guardrails::constants::{
    BEDROCK_BASE_BACKOFF_SECONDS, BEDROCK_MAX_THROTTLE_RETRIES,
};

use super::error::ApplyError;
use super::post::{PostCall, post};
use crate::bedrock_records::{ContentItem, Response, Usage};

// LiteLLM's _post_apply_guardrail_content_with_retry: a real throttle backs off at most three times
pub(super) async fn posted_with_retry<T>(
    call: &mut PostCall<'_>,
    content: &[ContentItem],
    transport: &T,
    completed: &mut Vec<Usage>,
) -> Result<Response, ApplyError>
where
    T: GuardrailTransport,
{
    let mut attempt: usize = 0;
    loop {
        match post(call, content, transport, completed).await {
            Ok(response) => return Ok(response),
            Err(error) if attempt < BEDROCK_MAX_THROTTLE_RETRIES && error.is_throttle() => {
                sleep_backoff(attempt).await;
            }
            Err(error) => return Err(error),
        }
        attempt = attempt.saturating_add(1);
    }
}

// the base delay doubles per attempt: 0.5s, then 1s, then 2s
async fn sleep_backoff(attempt: usize) {
    let exponent = u32::try_from(attempt).unwrap_or(u32::MAX);
    let factor = 2_u32.checked_pow(exponent).unwrap_or(1_u32);
    let delay = Duration::try_from_secs_f64(BEDROCK_BASE_BACKOFF_SECONDS)
        .unwrap_or_default()
        .checked_mul(factor)
        .unwrap_or_default();
    tokio::time::sleep(delay).await;
}
