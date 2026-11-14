#![cfg(test)]

use rstest::rstest;

use super::audio::estimate_audio_block_tokens;
use super::support::value;

#[rstest]
#[case::not_an_object(r#""audio""#, 300)]
#[case::missing_input_audio(r#"{"type":"input_audio"}"#, 300)]
#[case::missing_data(r#"{"type":"input_audio","input_audio":{}}"#, 300)]
#[case::empty_data(r#"{"type":"input_audio","input_audio":{"data":""}}"#, 300)]
#[case::tiny_payload(r#"{"type":"input_audio","input_audio":{"data":"AAAA"}}"#, 300)]
#[case::number_data(r#"{"type":"input_audio","input_audio":{"data":5}}"#, 300)]
fn small_or_missing_payloads_use_the_flat_estimate(#[case] block: &str, #[case] expected: i64) {
    assert_eq!(estimate_audio_block_tokens(&value(block)), expected);
}

fn payload_of(length: usize) -> String {
    format!(
        r#"{{"type":"input_audio","input_audio":{{"data":"{}"}}}}"#,
        "A".repeat(length)
    )
}

#[rstest]
#[case::at_the_floor(640_000, 300)]
#[case::one_token_past_the_floor(642_134, 301)]
#[case::a_million_characters(1_000_000, 468)]
fn a_large_payload_is_estimated_from_its_decoded_bytes(
    #[case] length: usize,
    #[case] expected: i64,
) {
    assert_eq!(
        estimate_audio_block_tokens(&value(&payload_of(length))),
        expected
    );
}
