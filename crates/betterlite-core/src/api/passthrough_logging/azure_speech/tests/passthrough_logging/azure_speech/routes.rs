#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::super::super::super::route::{SpeechRoute, classify_route};
use super::super::super::support;

const SHORT_AUDIO: &str = "https://eastus.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=en-US";
const BATCH: &str = "https://eastus.api.cognitive.microsoft.com/speechtotext/v3.2/transcriptions";
const FAST: &str = "https://eastus.api.cognitive.microsoft.com/speechtotext/transcriptions:transcribe?api-version=2024-11-15";
const PRICE: &str = r#"{"input_cost_per_second": 0.5}"#;

#[rstest]
#[case::short_audio(
    "/speech/recognition/conversation/cognitiveservices/v1",
    SpeechRoute::ShortAudio
)]
#[case::batch("/speechtotext/v3.2/transcriptions", SpeechRoute::Batch)]
#[case::fast(
    "/speechtotext/transcriptions:transcribe",
    SpeechRoute::FastTranscription
)]
#[case::short_audio_after_the_batch_prefix(
    "/speechtotext/v3.2/speech/recognition/x",
    SpeechRoute::ShortAudio
)]
#[case::batch_after_the_short_audio_prefix(
    "/speech/recognition/x/speechtotext/v3.2/transcriptions",
    SpeechRoute::Batch
)]
#[case::neither_prefix("/v1/audio/transcriptions", SpeechRoute::Batch)]
#[case::fast_with_a_trailing_slash("/speechtotext/transcriptions:transcribe/", SpeechRoute::Batch)]
#[case::fast_without_the_suffix("/speechtotext/transcriptions", SpeechRoute::Batch)]
fn the_path_classifies_the_route(#[case] path: &str, #[case] route: SpeechRoute) {
    assert_eq!(classify_route(path), route);
}

// The model name is what a user reads off the spend row, so the route table is checked through the handler too.
#[rstest]
#[case::short_audio(SHORT_AUDIO, "azure_speech/short-audio")]
#[case::batch(BATCH, "azure_speech/batch-transcription")]
#[case::batch_under_a_transcription_id(
    "https://eastus.api.cognitive.microsoft.com/speechtotext/v3.2/transcriptions/8a5d3f2c/files",
    "azure_speech/batch-transcription"
)]
#[case::fast(FAST, "azure_speech/fast-transcription")]
#[case::prefixed_short_audio(
    "https://apim.example.com/speech-proxy/speech/recognition/conversation/cognitiveservices/v1?language=en-US",
    "azure_speech/short-audio"
)]
#[case::prefixed_batch(
    "https://apim.example.com/speech/speechtotext/v3.2/transcriptions",
    "azure_speech/batch-transcription"
)]
#[case::prefixed_fast(
    "https://apim.example.com/speech/speechtotext/transcriptions:transcribe?api-version=2024-11-15",
    "azure_speech/fast-transcription"
)]
#[case::a_query_that_looks_like_the_short_audio_prefix(
    "https://eastus.api.cognitive.microsoft.com/speechtotext/v3.2/transcriptions?next=/speech/x",
    "azure_speech/batch-transcription"
)]
#[case::a_host_that_ends_with_the_fast_path(
    "https://speechtotext/transcriptions:transcribe",
    "azure_speech/batch-transcription"
)]
fn the_route_names_the_model(#[case] url_route: &str, #[case] model: &str) {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::priced(PRICE);
    let outcome = support::run(&logging_obj, &lookup, url_route, None, b"{}");
    assert_eq!(
        outcome.kwargs.entries.get("model"),
        Some(&Json::Text(model.to_owned()))
    );
}

// The spend row carries the provider prefix on every route, read off the same table the classifier returns.
#[test]
fn every_route_names_its_own_model() {
    assert_eq!(
        SpeechRoute::ShortAudio.model_name(),
        "azure_speech/short-audio"
    );
    assert_eq!(
        SpeechRoute::FastTranscription.model_name(),
        "azure_speech/fast-transcription"
    );
    assert_eq!(
        SpeechRoute::Batch.model_name(),
        "azure_speech/batch-transcription"
    );
}
