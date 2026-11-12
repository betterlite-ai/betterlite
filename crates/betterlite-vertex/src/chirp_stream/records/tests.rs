#![cfg(test)]

use google_cloud_speech_v2::model::{
    RecognitionResponseMetadata, SpeechRecognitionAlternative, StreamingRecognitionResult,
    StreamingRecognizeResponse, explicit_decoding_config::AudioEncoding,
    recognition_config::DecodingConfig, streaming_recognize_response::SpeechEventType,
};
use google_cloud_wkt::Duration as ProtoDuration;
use jiff::SignedDuration;
use rstest::rstest;

use super::{
    SpeechActivity, StreamingConfigure, billed_duration, cumulative_billed, first_request,
    response_event, speech_activity, streaming_config,
};

fn configure() -> StreamingConfigure {
    StreamingConfigure {
        model: "chirp_2".to_owned(),
        language_codes: vec!["en-US".to_owned(), "fr-FR".to_owned()],
        sample_rate_hertz: 24_000,
    }
}

fn result(transcripts: &[&str], is_final: bool) -> StreamingRecognitionResult {
    let alternatives: Vec<SpeechRecognitionAlternative> = transcripts
        .iter()
        .map(|transcript| SpeechRecognitionAlternative::new().set_transcript(*transcript))
        .collect();
    StreamingRecognitionResult::new()
        .set_alternatives(alternatives)
        .set_is_final(is_final)
}

fn response(
    event: SpeechEventType,
    results: Vec<StreamingRecognitionResult>,
    billed: Option<ProtoDuration>,
) -> StreamingRecognizeResponse {
    let mut response = StreamingRecognizeResponse::new()
        .set_speech_event_type(event)
        .set_results(results);
    if let Some(duration) = billed {
        response = response
            .set_metadata(RecognitionResponseMetadata::new().set_total_billed_duration(duration));
    }
    response
}

#[test]
fn config_first_request_carries_the_recognizer_and_streaming_config() {
    let config = streaming_config(&configure());
    let body = config.config.as_ref().expect("a recognition config");
    assert_eq!(body.model, "chirp_2");
    assert_eq!(body.language_codes, ["en-US", "fr-FR"]);
    let DecodingConfig::ExplicitDecodingConfig(decoding) =
        body.decoding_config.as_ref().expect("explicit decoding")
    else {
        panic!("the first request must decode explicitly");
    };
    assert_eq!(decoding.encoding, AudioEncoding::Linear16);
    assert_eq!(decoding.sample_rate_hertz, 24_000_i32);
    assert_eq!(decoding.audio_channel_count, 1_i32);
    let features = config
        .streaming_features
        .as_ref()
        .expect("streaming features");
    assert!(features.interim_results);
    assert!(features.enable_voice_activity_events);
    let request = first_request(
        "projects/project/locations/us-central1/recognizers/_",
        config,
    );
    assert_eq!(
        request.recognizer,
        "projects/project/locations/us-central1/recognizers/_"
    );
    assert!(request.streaming_config().is_some());
    assert!(request.audio().is_none());
}

#[test]
fn no_alternative_projects_empty_text() {
    let projection = response_event(
        &response(SpeechEventType::Unspecified, vec![result(&[], true)], None),
        0.0,
    );
    assert_eq!(projection.results.len(), 1);
    assert_eq!(projection.results[0].transcript, "");
    assert!(projection.results[0].is_final);
}

#[test]
fn the_first_alternative_wins() {
    let projection = response_event(
        &response(
            SpeechEventType::Unspecified,
            vec![result(&["hello", "hullo"], false)],
            None,
        ),
        1.5,
    );
    assert_eq!(projection.results[0].transcript, "hello");
    assert!(!projection.results[0].is_final);
    assert_eq!(projection.billed_seconds.to_bits(), 1.5_f64.to_bits());
}

#[rstest]
#[case::begin(SpeechEventType::SpeechActivityBegin, SpeechActivity::Begin)]
#[case::activity_end(SpeechEventType::SpeechActivityEnd, SpeechActivity::End)]
#[case::end_of_utterance(SpeechEventType::EndOfSingleUtterance, SpeechActivity::End)]
#[case::unspecified(SpeechEventType::Unspecified, SpeechActivity::None)]
fn activity_events_map_to_their_projection(
    #[case] event: SpeechEventType,
    #[case] activity: SpeechActivity,
) {
    let response = response(event, Vec::new(), None);
    assert_eq!(speech_activity(&response), activity);
    assert_eq!(response_event(&response, 0.0).speech_event, activity);
}

#[test]
fn an_unknown_activity_name_projects_none() {
    let response = response(
        SpeechEventType::from("SPEECH_ACTIVITY_UNKNOWN"),
        Vec::new(),
        None,
    );
    assert_eq!(speech_activity(&response), SpeechActivity::None);
    assert_eq!(
        response_event(&response, 0.0).speech_event,
        SpeechActivity::None
    );
}

#[test]
fn billed_duration_keeps_exact_seconds_and_nanos() {
    let duration = billed_duration(&response(
        SpeechEventType::Unspecified,
        Vec::new(),
        Some(ProtoDuration::new(2, 250_000_000).expect("a valid duration")),
    ));
    assert_eq!(duration, SignedDuration::new(2, 250_000_000));
}

#[test]
fn a_missing_billed_duration_is_zero() {
    assert_eq!(
        billed_duration(&response(SpeechEventType::Unspecified, Vec::new(), None)),
        SignedDuration::ZERO
    );
    let metadata_without_duration =
        StreamingRecognizeResponse::new().set_metadata(RecognitionResponseMetadata::new());
    assert_eq!(
        billed_duration(&metadata_without_duration),
        SignedDuration::ZERO
    );
}

#[test]
fn cumulative_billing_adds_the_stream_maximum() {
    assert_eq!(
        cumulative_billed(0.5, SignedDuration::new(1, 250_000_000)).to_bits(),
        1.75_f64.to_bits()
    );
}

#[test]
fn the_event_serializes_as_the_wire_model() {
    let event = response_event(
        &response(
            SpeechEventType::SpeechActivityBegin,
            vec![result(&["hello"], true)],
            None,
        ),
        0.0,
    );
    assert_eq!(
        serde_json::to_string(&event).expect("the event serializes"),
        r#"{"kind":"response","speech_event":"begin","results":[{"transcript":"hello","is_final":true}],"billed_seconds":0.0}"#
    );
}
