use std::cmp::Ordering;
use std::mem;

use betterlite_models::catalog::ModelCatalog;
use serde::Serialize;

use super::error::LiveAccountingError;
use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_live_setup::catalog::is_text_only;

const PCM16_BYTES_PER_SECOND: u128 = 48_000;
const AUDIO_TOKENS_PER_SECOND: u128 = 25;
const OUTPUT_TEXT_TOKENS_PER_MINUTE: u128 = 175;
// the output rate is per minute, so its divisor is 48000 bytes per second times 60 seconds
const OUTPUT_DIVISOR: u128 = 2_880_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptionUsage {
    #[serde(rename = "type")]
    pub usage_type: &'static str,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub input_token_details: TranscriptionInputDetails,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptionInputDetails {
    pub text_tokens: i64,
    pub audio_tokens: i64,
}

// Live sends no usageMetadata for transcribe sessions, so billing comes from the streamed audio duration
pub fn consume_transcript_estimate(
    state: &mut LiveSessionState,
    catalog: &ModelCatalog,
    model: &str,
) -> Result<Option<TranscriptionUsage>, LiveAccountingError> {
    if state.unbilled_input_audio_bytes == 0 || !is_text_only(catalog, model) {
        return Ok(None);
    }
    let bytes = u128::from(mem::take(&mut state.unbilled_input_audio_bytes));
    let audio = round_half_even(
        bytes.saturating_mul(AUDIO_TOKENS_PER_SECOND),
        PCM16_BYTES_PER_SECOND,
    );
    let output = round_half_even(
        bytes.saturating_mul(OUTPUT_TEXT_TOKENS_PER_MINUTE),
        OUTPUT_DIVISOR,
    );
    Ok(Some(TranscriptionUsage {
        usage_type: "tokens",
        input_tokens: i64::try_from(audio)?,
        output_tokens: i64::try_from(output)?,
        total_tokens: i64::try_from(audio.saturating_add(output))?,
        input_token_details: TranscriptionInputDetails {
            text_tokens: 0,
            audio_tokens: i64::try_from(audio)?,
        },
    }))
}

// Python's round() sends an exact tie to the even neighbour, and the ratio stays exact here instead of passing through a float
fn round_half_even(numerator: u128, denominator: u128) -> u128 {
    let quotient = numerator.div_euclid(denominator);
    let doubled_remainder = numerator.rem_euclid(denominator).saturating_mul(2);
    let round_up = match doubled_remainder.cmp(&denominator) {
        Ordering::Greater => true,
        Ordering::Equal => quotient.rem_euclid(2) == 1,
        Ordering::Less => false,
    };
    if round_up {
        quotient.saturating_add(1)
    } else {
        quotient
    }
}
