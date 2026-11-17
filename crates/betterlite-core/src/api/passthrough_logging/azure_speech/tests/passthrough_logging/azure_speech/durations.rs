#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::super::super::super::AzureSpeechLoggingError;
use super::super::super::super::duration::billed_audio_seconds;
use super::super::super::super::route::SpeechRoute;
use super::super::super::support;

const RECOGNIZED: &[u8] = br#"{"Offset": 5000000, "Duration": 25000000}"#;
// The transcribe crate's one-second FLAC, OGG and MP3 fixtures, the containers only a format-aware decoder measures.
const VALID_FLAC: &[u8] = include_bytes!(
    "../../../../../../../../betterlite-transcribe/fuzz/corpus/media_duration/valid.flac"
);
const VALID_OGG: &[u8] = include_bytes!(
    "../../../../../../../../betterlite-transcribe/fuzz/corpus/media_duration/valid.ogg"
);
const VALID_MP3: &[u8] = include_bytes!(
    "../../../../../../../../betterlite-transcribe/fuzz/corpus/media_duration/valid.mp3"
);

#[test]
fn short_audio_bills_the_recognized_ticks() {
    let seconds = billed_audio_seconds(SpeechRoute::ShortAudio, None, RECOGNIZED)
        .expect("the tick fields derive a duration");
    assert_eq!(seconds, support::dec("3"));
}

#[test]
fn short_audio_bills_the_longer_of_the_uploaded_and_the_recognized_audio() {
    let short_upload = support::wav_seconds(2);
    let long_upload = support::wav_seconds(4);
    assert_eq!(
        billed_audio_seconds(
            SpeechRoute::ShortAudio,
            Some(short_upload.as_slice()),
            RECOGNIZED
        )
        .expect("the duration derives"),
        support::dec("3")
    );
    assert_eq!(
        billed_audio_seconds(
            SpeechRoute::ShortAudio,
            Some(long_upload.as_slice()),
            RECOGNIZED
        )
        .expect("the duration derives"),
        support::dec("4")
    );
}

#[test]
fn short_audio_bills_the_upload_when_nothing_was_recognized() {
    let upload = support::wav_seconds(2);
    assert_eq!(
        billed_audio_seconds(
            SpeechRoute::ShortAudio,
            Some(upload.as_slice()),
            br#"{"RecognitionStatus":"NoMatch"}"#
        )
        .expect("the duration derives"),
        support::dec("2")
    );
}

#[rstest]
#[case::empty_object(b"{}")]
#[case::list(b"[]")]
#[case::null(b"null")]
#[case::text(b"\"Success\"")]
#[case::empty_body(b"")]
#[case::not_json(b"not json")]
#[case::zero_ticks(br#"{"Offset": 0, "Duration": 0}"#)]
#[case::string_ticks(br#"{"Offset": "5000000", "Duration": "25000000"}"#)]
#[case::float_ticks(br#"{"Offset": 5000000.0, "Duration": 25000000.0}"#)]
#[case::a_missing_offset(br#"{"Duration": 25000000}"#)]
#[case::a_missing_duration(br#"{"Offset": 5000000}"#)]
#[case::null_ticks(br#"{"Offset": null, "Duration": null}"#)]
#[case::structured_ticks(br#"{"Offset": {"start": 5000000}, "Duration": [25000000]}"#)]
fn short_audio_without_usable_ticks_bills_no_recognition(#[case] response_body: &[u8]) {
    assert_eq!(
        billed_audio_seconds(SpeechRoute::ShortAudio, None, response_body)
            .expect("an unusable body is no duration"),
        Decimal::ZERO
    );
}

// Python's isinstance(x, int) is true for a bool, so a boolean tick counts as zero or one.
#[test]
fn a_boolean_tick_counts_as_zero_or_one() {
    assert_eq!(
        billed_audio_seconds(
            SpeechRoute::ShortAudio,
            None,
            br#"{"Offset": true, "Duration": true}"#
        )
        .expect("the tick fields derive a duration"),
        support::dec("0.0000002")
    );
    assert_eq!(
        billed_audio_seconds(
            SpeechRoute::ShortAudio,
            None,
            br#"{"Offset": false, "Duration": 25000000}"#
        )
        .expect("the tick fields derive a duration"),
        support::dec("2.5")
    );
}

#[test]
fn an_upload_that_is_not_audio_bills_nothing() {
    assert_eq!(
        billed_audio_seconds(
            SpeechRoute::ShortAudio,
            Some(b"not audio at all".as_slice()),
            br#"{"RecognitionStatus":"InitialSilenceTimeout"}"#
        )
        .expect("an undecodable upload is no duration"),
        Decimal::ZERO
    );
}

// A FLAC, OGG or MP3 upload is a complete one-second container the WAV decoder does not read, so it adds no uploaded seconds.
#[test]
fn a_foreign_container_upload_bills_nothing() {
    let unrecognized = br#"{"RecognitionStatus":"Success"}"#;
    assert_eq!(
        billed_audio_seconds(SpeechRoute::ShortAudio, Some(VALID_FLAC), unrecognized)
            .expect("a FLAC upload is no duration"),
        Decimal::ZERO
    );
    assert_eq!(
        billed_audio_seconds(SpeechRoute::ShortAudio, Some(VALID_OGG), unrecognized)
            .expect("an OGG upload is no duration"),
        Decimal::ZERO
    );
    assert_eq!(
        billed_audio_seconds(SpeechRoute::ShortAudio, Some(VALID_MP3), unrecognized)
            .expect("an MP3 upload is no duration"),
        Decimal::ZERO
    );
}

#[test]
fn a_negative_recognition_result_never_subtracts_from_the_upload() {
    let negative = br#"{"Offset": -25000000, "Duration": 5000000}"#;
    let upload = support::wav_seconds(2);
    assert_eq!(
        billed_audio_seconds(SpeechRoute::ShortAudio, Some(upload.as_slice()), negative)
            .expect("the duration derives"),
        support::dec("2")
    );
    assert_eq!(
        billed_audio_seconds(SpeechRoute::ShortAudio, None, negative)
            .expect("the duration derives"),
        Decimal::ZERO
    );
}

#[rstest]
#[case::milliseconds(br#"{"durationMilliseconds": 5061}"#, "5.061")]
#[case::zero(br#"{"durationMilliseconds": 0}"#, "0")]
#[case::string(br#"{"durationMilliseconds": "5061"}"#, "0")]
#[case::float(br#"{"durationMilliseconds": 5061.0}"#, "0")]
#[case::another_key(br#"{"duration": 5061}"#, "0")]
#[case::list(b"[]", "0")]
#[case::empty_object(b"{}", "0")]
#[case::not_json(b"not json", "0")]
fn fast_transcription_bills_the_reported_milliseconds(
    #[case] response_body: &[u8],
    #[case] seconds: &str,
) {
    assert_eq!(
        billed_audio_seconds(SpeechRoute::FastTranscription, None, response_body)
            .expect("an unusable body is no duration"),
        support::dec(seconds)
    );
}

#[test]
fn fast_transcription_ignores_the_uploaded_audio() {
    let upload = support::wav_seconds(30);
    assert_eq!(
        billed_audio_seconds(
            SpeechRoute::FastTranscription,
            Some(upload.as_slice()),
            br#"{"durationMilliseconds": 5061}"#
        )
        .expect("the duration derives"),
        support::dec("5.061")
    );
}

#[test]
fn batch_transcription_bills_nothing_from_the_body_or_the_upload() {
    let upload = support::wav_seconds(30);
    assert_eq!(
        billed_audio_seconds(
            SpeechRoute::Batch,
            Some(upload.as_slice()),
            br#"{"durationMilliseconds": 5061, "Offset": 5000000, "Duration": 25000000}"#
        )
        .expect("a batch route has no duration"),
        Decimal::ZERO
    );
}

#[test]
fn a_tick_total_beyond_the_decimal_range_is_an_error() {
    let body = format!(r#"{{"Offset": {}, "Duration": 1}}"#, Decimal::MAX);
    let error = billed_audio_seconds(SpeechRoute::ShortAudio, None, body.as_bytes())
        .expect_err("the tick total leaves the decimal range");
    assert!(matches!(error, AzureSpeechLoggingError::DurationRange));
}

#[test]
fn a_tick_value_beyond_the_decimal_range_is_an_error() {
    let error = billed_audio_seconds(
        SpeechRoute::ShortAudio,
        None,
        br#"{"Offset": 10000000000000000000000000000000000000000, "Duration": 1}"#,
    )
    .expect_err("the tick value leaves the decimal range");
    assert!(matches!(
        error,
        AzureSpeechLoggingError::FieldValue {
            field: "Offset",
            ..
        }
    ));
}

// The fast route reads its own field, so the error has to name that field rather than a tick.
#[test]
fn a_millisecond_value_beyond_the_decimal_range_names_its_field() {
    let error = billed_audio_seconds(
        SpeechRoute::FastTranscription,
        None,
        br#"{"durationMilliseconds": 10000000000000000000000000000000000000000}"#,
    )
    .expect_err("the milliseconds leave the decimal range");
    assert!(matches!(
        error,
        AzureSpeechLoggingError::FieldValue {
            field: "durationMilliseconds",
            ..
        }
    ));
}
