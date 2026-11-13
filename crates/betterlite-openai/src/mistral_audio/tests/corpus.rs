use std::fs;
use std::path::{Path, PathBuf};

use betterlite_http::proxy::UpstreamResponse;
use serde_json::Value;

use super::super::error::MistralAudioError;
use super::super::speech::decode_voxtral_reply;
use super::super::transcription::{VoxtralTranscription, decode_voxtral_transcription_reply};
use super::{response, speech_request};
use crate::files::records::BinaryContent;

const UPSTREAM_STATUS: u16 = 502;

fn category(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .expect("the seed name is UTF-8")
        .to_owned()
}

fn upstream(bytes: &[u8]) -> UpstreamResponse {
    response(UPSTREAM_STATUS, &[], bytes)
}

fn audio_seed(bytes: &[u8]) -> Result<BinaryContent, MistralAudioError> {
    decode_voxtral_reply(
        &speech_request(&[("response_format", "wav")]),
        &upstream(bytes),
    )
}

fn transcription_seed(bytes: &[u8]) -> Result<VoxtralTranscription, MistralAudioError> {
    decode_voxtral_transcription_reply(&upstream(bytes))
}

fn is_not_json(error: &MistralAudioError) -> bool {
    matches!(error, MistralAudioError::ReplyNotJson { status } if *status == UPSTREAM_STATUS)
}

fn valid_audio(bytes: &[u8]) -> bool {
    matches!(audio_seed(bytes), Ok(content) if content.status == 200)
}

fn bad_audio(bytes: &[u8]) -> bool {
    matches!(
        audio_seed(bytes),
        Err(MistralAudioError::AudioNotBase64 { .. })
    )
}

fn missing_audio(bytes: &[u8]) -> bool {
    matches!(audio_seed(bytes), Err(MistralAudioError::AudioDataMissing))
        && matches!(transcription_seed(bytes), Ok(decoded) if decoded.response.text.as_deref() == Some("hola"))
}

fn empty_audio(bytes: &[u8]) -> bool {
    matches!(audio_seed(bytes), Err(MistralAudioError::AudioDataMissing))
        && matches!(transcription_seed(bytes), Ok(decoded) if decoded.response.text.as_deref() == Some(""))
}

fn not_an_object(bytes: &[u8]) -> bool {
    matches!(audio_seed(bytes), Err(MistralAudioError::ReplyNotObject))
        && matches!(
            transcription_seed(bytes),
            Err(MistralAudioError::ReplyNotObject)
        )
}

fn not_json(bytes: &[u8]) -> bool {
    audio_seed(bytes).as_ref().err().is_some_and(is_not_json)
        && transcription_seed(bytes)
            .as_ref()
            .err()
            .is_some_and(is_not_json)
}

fn diarized(bytes: &[u8]) -> bool {
    matches!(audio_seed(bytes), Err(MistralAudioError::AudioDataMissing))
        && matches!(
            &transcription_seed(bytes),
            Ok(decoded)
                if decoded.response.text.as_deref() == Some("hola")
                    && decoded.response.extra.contains_key("segments")
                    && decoded.response.extra.contains_key("language")
        )
}

fn null_fields(bytes: &[u8]) -> bool {
    matches!(audio_seed(bytes), Err(MistralAudioError::AudioDataMissing))
        && matches!(
            &transcription_seed(bytes),
            Ok(decoded)
                if decoded.response.text.as_deref() == Some("")
                    && decoded.response.extra.get("segments") == Some(&Value::Null)
                    && decoded.response.extra.get("language") == Some(&Value::Null)
        )
}

fn number_text(bytes: &[u8]) -> bool {
    matches!(audio_seed(bytes), Err(MistralAudioError::AudioDataMissing))
        && matches!(
            transcription_seed(bytes),
            Err(MistralAudioError::TextNotString)
        )
}

fn empty_transcription(bytes: &[u8]) -> bool {
    matches!(audio_seed(bytes), Err(MistralAudioError::AudioDataMissing))
        && matches!(transcription_seed(bytes), Ok(decoded) if decoded.response.text.as_deref() == Some(""))
}

fn replays(path: &Path) -> bool {
    let bytes = fs::read(path).expect("the seed reads");
    match category(path).as_str() {
        "speech-valid" | "speech-trailing-bits" => valid_audio(&bytes),
        "speech-invalid-base64" | "speech-bad-padding" => bad_audio(&bytes),
        "speech-missing-audio" => missing_audio(&bytes),
        "speech-empty-audio" => empty_audio(&bytes),
        "speech-non-object" | "transcription-non-object" => not_an_object(&bytes),
        "speech-malformed" | "speech-non-json" | "transcription-malformed" => not_json(&bytes),
        "transcription-diarization" => diarized(&bytes),
        "transcription-null-fields" => null_fields(&bytes),
        "transcription-number-text" => number_text(&bytes),
        "transcription-empty" => empty_transcription(&bytes),
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_voxtral_reply_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/voxtral_reply");
    let seeds: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("the corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    assert!(!seeds.is_empty(), "the corpus has seeds");
    assert_eq!(
        seeds
            .iter()
            .find(|path| !replays(path))
            .map(PathBuf::as_path),
        None
    );
}
