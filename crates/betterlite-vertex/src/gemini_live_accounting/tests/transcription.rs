#![cfg(test)]

use rstest::rstest;

use super::support::{LIVE_MODEL, TRANSCRIBE_MODEL, catalog};
use crate::gemini_live_accounting::transcription::{
    TranscriptionInputDetails, TranscriptionUsage, consume_transcript_estimate,
};
use crate::gemini_live_client::state::{LiveSessionState, ToolCallNames};

fn state_with(bytes: u64) -> LiveSessionState {
    LiveSessionState {
        tool_call_names: ToolCallNames::default(),
        pending_usage_metadata: None,
        unbilled_input_audio_bytes: bytes,
    }
}

fn estimate(state: &mut LiveSessionState, model: &str) -> Option<TranscriptionUsage> {
    consume_transcript_estimate(state, &catalog(), model).expect("the estimate fits")
}

#[rstest]
#[case::one_second(48_000, 25, 3)]
#[case::audio_tie_rounds_down_to_zero(960, 0, 0)]
#[case::audio_exact(1_920, 1, 0)]
#[case::audio_tie_rounds_up_to_two(2_880, 2, 0)]
#[case::audio_tie_rounds_down_to_two(4_800, 2, 0)]
#[case::output_tie_rounds_up_to_four(57_600, 30, 4)]
#[case::output_tie_rounds_down_to_ten(172_800, 90, 10)]
#[case::output_just_below_a_half(8_228, 4, 0)]
#[case::output_just_above_a_half(8_229, 4, 1)]
#[case::one_byte(1, 0, 0)]
#[case::audio_half_below_odd(24_000, 12, 1)]
#[case::a_minute_of_audio(2_880_000, 1_500, 175)]
fn the_estimate_rounds_exact_ratios_to_the_even_neighbour(
    #[case] bytes: u64,
    #[case] audio_tokens: i64,
    #[case] output_tokens: i64,
) {
    let mut state = state_with(bytes);
    let usage = estimate(&mut state, TRANSCRIBE_MODEL).expect("a transcribe session bills");
    assert_eq!(
        usage,
        TranscriptionUsage {
            usage_type: "tokens",
            input_tokens: audio_tokens,
            output_tokens,
            total_tokens: audio_tokens + output_tokens,
            input_token_details: TranscriptionInputDetails {
                text_tokens: 0,
                audio_tokens,
            },
        }
    );
}

#[test]
fn estimating_resets_the_unbilled_bytes() {
    let mut state = state_with(48_000);
    estimate(&mut state, TRANSCRIBE_MODEL);
    assert_eq!(state.unbilled_input_audio_bytes, 0);
}

#[test]
fn a_second_close_has_nothing_left_to_bill() {
    let mut state = state_with(48_000);
    assert!(estimate(&mut state, TRANSCRIBE_MODEL).is_some());
    assert_eq!(estimate(&mut state, TRANSCRIBE_MODEL), None);
}

#[test]
fn no_unbilled_audio_means_no_estimate() {
    let mut state = state_with(0);
    assert_eq!(estimate(&mut state, TRANSCRIBE_MODEL), None);
}

#[rstest]
#[case::a_live_model(LIVE_MODEL)]
#[case::an_unlisted_model("not-in-the-catalog")]
fn a_model_that_is_not_transcription_keeps_its_bytes(#[case] model: &str) {
    let mut state = state_with(48_000);
    assert_eq!(estimate(&mut state, model), None);
    assert_eq!(state.unbilled_input_audio_bytes, 48_000);
}

#[test]
fn the_largest_byte_count_still_fits_the_token_fields() {
    let mut state = state_with(u64::MAX);
    let usage = estimate(&mut state, TRANSCRIBE_MODEL).expect("a transcribe session bills");
    assert_eq!(usage.input_tokens, 9_607_679_205_057_058);
    assert_eq!(usage.output_tokens, 1_120_895_907_256_657);
    assert_eq!(usage.total_tokens, 10_728_575_112_313_715);
}
