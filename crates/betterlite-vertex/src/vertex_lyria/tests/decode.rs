#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_models::catalog::BundledCatalog;
use rstest::rstest;

use super::super::audio::{DEFAULT_SPEECH_MEDIA_TYPE, LyriaDecode, decode_lyria_audio};
use super::super::error::VertexLyriaError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::model_policy::lyria::LyriaOverlay;
use crate::vertex_speech::audio::BinaryAudio;

fn decode(model: &str, body: &str) -> Result<BinaryAudio, VertexLyriaError> {
    with_catalog!(|overlay| {
        let decode = LyriaDecode {
            model,
            status: 200,
            body: body.as_bytes(),
        };
        decode_lyria_audio(overlay, &decode, &sniff_flac)
    })
}

fn sniff_flac(audio: &[u8]) -> Option<String> {
    audio.starts_with(b"fLaC").then(|| "audio/flac".to_owned())
}

#[test]
fn decodes_the_first_prediction_only() {
    let body = r#"{"predictions": [
        {"audioContent": "QUJD", "mimeType": "audio/wav"},
        {"audioContent": "WFk=", "mimeType": "audio/mpeg"}
    ]}"#;
    let audio = decode("lyria-002", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
    assert_eq!(audio.status, 200);
}

#[test]
fn prefers_audio_content_over_the_base64_field() {
    let body = r#"{"predictions": [{"audioContent": "QUJD", "bytesBase64Encoded": "WFk="}]}"#;
    assert_eq!(decode("lyria-002", body).unwrap().bytes.as_slice(), b"ABC");
}

#[test]
fn falls_back_to_the_base64_field() {
    let body = r#"{"predictions": [{"bytesBase64Encoded": "WFk="}]}"#;
    assert_eq!(decode("lyria-002", body).unwrap().bytes.as_slice(), b"XY");
}

#[test]
fn falls_back_to_the_base64_field_when_audio_content_is_empty() {
    let body = r#"{"predictions": [{"audioContent": "", "bytesBase64Encoded": "QUJD"}]}"#;
    assert_eq!(decode("lyria-002", body).unwrap().bytes.as_slice(), b"ABC");
}

#[test]
fn ignores_characters_outside_the_base64_alphabet() {
    let body = r#"{"predictions": [{"audioContent": "@@@"}]}"#;
    let audio = decode("lyria-002", body).unwrap();
    assert!(audio.bytes.is_empty());
    assert_eq!(
        audio.content_type.as_deref(),
        Some(DEFAULT_SPEECH_MEDIA_TYPE)
    );
}

#[test]
fn ignores_the_later_predictions() {
    let body = r#"{"predictions": [{"mimeType": "audio/wav"}, {"audioContent": "QUJD"}]}"#;
    let error = decode("lyria-002", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::MissingAudio { .. }));
}

#[rstest]
#[case::no_predictions("{}")]
#[case::empty_predictions(r#"{"predictions": []}"#)]
#[case::null_predictions(r#"{"predictions": null}"#)]
#[case::no_audio(r#"{"predictions": [{"mimeType": "audio/wav"}]}"#)]
#[case::empty_audio(r#"{"predictions": [{"audioContent": ""}]}"#)]
#[case::null_audio(r#"{"predictions": [{"audioContent": null}]}"#)]
fn rejects_a_prediction_without_audio(#[case] body: &str) {
    let error = decode("lyria-002", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::MissingAudio { .. }));
    assert_eq!(error.status(), Some(500));
}

#[test]
fn names_the_prefix_stripped_model_in_the_missing_audio_error() {
    let error = decode("vertex_ai/lyria-002", "{}").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no generated audio found in Vertex AI lyria-002 response")
    );
}

#[test]
fn falls_back_to_the_sniffer_for_an_empty_mime_type() {
    let body = r#"{"predictions": [{"audioContent": "ZkxhQwA=", "mimeType": ""}]}"#;
    let audio = decode("lyria-002", body).unwrap();
    assert_eq!(audio.content_type.as_deref(), Some("audio/flac"));
}

#[test]
fn falls_back_to_the_provider_mime_type() {
    let body = r#"{"predictions": [{"audioContent": "ZkxhQwA=", "mimeType": "audio/wav"}]}"#;
    let audio = decode("lyria-002", body).unwrap();
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn keeps_the_provider_mime_type_on_an_interaction_audio_item() {
    let body = r#"{"steps": [{"type": "audio", "data": "ZkxhQwA=", "mime_type": "audio/wav"}]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn defaults_to_audio_mpeg_without_a_sniff_match() {
    let body = r#"{"predictions": [{"audioContent": "QUJD"}]}"#;
    let audio = decode("lyria-002", body).unwrap();
    assert_eq!(
        audio.content_type.as_deref(),
        Some(DEFAULT_SPEECH_MEDIA_TYPE)
    );
}

#[test]
fn preserves_the_upstream_status() {
    let body = r#"{"predictions": [{"audioContent": "QUJD"}]}"#;
    let audio = with_catalog!(|overlay| {
        let decode = LyriaDecode {
            model: "lyria-002",
            status: 503,
            body: body.as_bytes(),
        };
        decode_lyria_audio(overlay, &decode, &sniff_flac)
    })
    .unwrap();
    assert_eq!(audio.status, 503);
}

#[test]
fn rejects_an_unknown_model_when_decoding() {
    let error = decode("lyria-9", "{}").unwrap_err();
    assert!(matches!(error, VertexLyriaError::UnknownModel { .. }));
}

#[test]
fn rejects_a_response_over_the_limit() {
    let body = vec![b' '; MAX_RESPONSE_BYTES.saturating_add(1)];
    let error = with_catalog!(|overlay| {
        let decode = LyriaDecode {
            model: "lyria-002",
            status: 200,
            body: body.as_slice(),
        };
        decode_lyria_audio(overlay, &decode, &sniff_flac)
    })
    .unwrap_err();
    assert!(matches!(error, VertexLyriaError::ResponseTooLarge { .. }));
}

#[test]
fn keeps_a_body_of_exactly_the_limit() {
    let body = vec![b' '; MAX_RESPONSE_BYTES];
    let error = with_catalog!(|overlay| {
        let decode = LyriaDecode {
            model: "lyria-002",
            status: 200,
            body: body.as_slice(),
        };
        decode_lyria_audio(overlay, &decode, &sniff_flac)
    })
    .unwrap_err();
    assert!(matches!(error, VertexLyriaError::ResponseJson(_)));
}

#[rstest]
#[case::malformed("{nope")]
#[case::list("[]")]
#[case::text(r#""text""#)]
fn rejects_a_body_that_is_not_a_response_object(#[case] body: &str) {
    let error = decode("lyria-002", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::ResponseJson(_)));
}

#[test]
fn takes_the_last_audio_of_the_steps() {
    let body = r#"{"steps": [
        {"type": "audio", "data": "QUJD", "mime_type": "audio/mp3"},
        {"type": "audio", "data": "WFk=", "mime_type": "audio/wav"}
    ]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"XY");
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn scans_outputs_when_steps_is_absent() {
    let body = r#"{"outputs": [{"type": "audio", "data": "QUJD"}]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
    assert_eq!(
        audio.content_type.as_deref(),
        Some(DEFAULT_SPEECH_MEDIA_TYPE)
    );
}

#[test]
fn scans_outputs_when_steps_is_empty() {
    let body = r#"{"steps": [], "outputs": [{"type": "audio", "data": "QUJD"}]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
}

#[test]
fn prefers_steps_over_outputs() {
    let body = r#"{
        "steps": [{"type": "audio", "data": "QUJD"}],
        "outputs": [{"type": "audio", "data": "WFk="}]
    }"#;
    assert_eq!(
        decode("lyria-3-pro-preview", body)
            .unwrap()
            .bytes
            .as_slice(),
        b"ABC"
    );
}

#[test]
fn keeps_the_steps_when_they_carry_no_audio() {
    let body = r#"{
        "steps": [{"type": "text", "data": "QUJD"}],
        "outputs": [{"type": "audio", "data": "WFk="}]
    }"#;
    let error = decode("lyria-3-pro-preview", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::MissingAudio { .. }));
}

#[test]
fn descends_the_content_of_a_model_output_step() {
    let body = r#"{"steps": [{
        "type": "model_output",
        "content": [
            {"type": "text", "data": "ignored"},
            {"type": "audio", "data": "QUJD", "mime_type": "audio/wav"}
        ]
    }]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn treats_a_step_with_another_type_as_its_own_audio_item() {
    let body = r#"{"steps": [{"type": "audio", "data": "QUJD", "mime_type": "audio/wav"}]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"ABC");
    assert_eq!(audio.content_type.as_deref(), Some("audio/wav"));
}

#[test]
fn ignores_the_data_of_a_model_output_step() {
    let body = r#"{"steps": [{"type": "model_output", "data": "QUJD"}]}"#;
    let error = decode("lyria-3-pro-preview", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::MissingAudio { .. }));
}

#[test]
fn skips_items_that_are_not_audio_or_carry_no_data() {
    let body = r#"{"steps": [
        {"type": "text", "data": "QUJD"},
        {"type": "audio", "data": ""},
        {"type": "audio"},
        {"type": "model_output", "content": [{"type": "text", "data": "WFk="}]}
    ]}"#;
    let error = decode("lyria-3-pro-preview", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::MissingAudio { .. }));
}

#[test]
fn resets_the_mime_type_with_the_last_audio() {
    let body = r#"{"steps": [
        {"type": "audio", "data": "QUJD", "mime_type": "audio/mp3"},
        {"type": "audio", "data": "ZkxhQwA="}
    ]}"#;
    let audio = decode("lyria-3-pro-preview", body).unwrap();
    assert_eq!(audio.bytes.as_slice(), b"fLaC\x00");
    assert_eq!(audio.content_type.as_deref(), Some("audio/flac"));
}

#[rstest]
#[case::empty("{}")]
#[case::empty_steps(r#"{"steps": []}"#)]
#[case::null_steps(r#"{"steps": null}"#)]
#[case::audio_without_data(r#"{"steps": [{"type": "audio"}]}"#)]
fn rejects_an_interaction_response_without_audio(#[case] body: &str) {
    let error = decode("lyria-3-pro-preview", body).unwrap_err();
    assert!(matches!(error, VertexLyriaError::MissingAudio { .. }));
}

#[test]
fn uses_the_interactions_dialect_for_a_prefixed_model() {
    let body = r#"{"outputs": [{"type": "audio", "data": "QUJD"}]}"#;
    assert_eq!(
        decode("vertex_ai/lyria-3-pro-preview", body)
            .unwrap()
            .bytes
            .as_slice(),
        b"ABC"
    );
}

#[rstest]
fn replays_the_lyria_audio_corpus(#[files("fuzz/corpus/lyria_audio/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    with_catalog!(|overlay| {
        for model in ["lyria-002", "lyria-3-pro-preview"] {
            let decode = LyriaDecode {
                model,
                status: 200,
                body: &bytes,
            };
            drop(decode_lyria_audio(overlay, &decode, &sniff_flac));
        }
    });
}
