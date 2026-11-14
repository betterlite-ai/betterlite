#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_studio_transcription() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("studio_transcription"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "studio_transcription"
    }));
}

// the expected answers are LiteLLM's own, captured from the Python helper on the same case
#[rstest]
#[case::bare_audio_request(
    "bare-audio-request.yaml",
    r#"[["language","response_format","timestamp_granularities"],true,{"model":"gemini-3.5-transcribe","input":[{"type":"audio","data":"YXVkaW8=","mime_type":"audio/wav"}]},{"response_format":"json","unsupported_option":1,"language":"en"}]"#
)]
#[case::subtitle_implies_diarization(
    "subtitle-implies-diarization.yaml",
    r#"[{"model":"gemini-3.5-transcribe","input":[{"type":"audio","data":"YXVkaW8=","mime_type":"audio/wav"}],"generation_config":{"transcription_config":{"mode":{"type":"verbatim","timestamp_granularities":["word"],"diarization_mode":"speaker"}}}},{"model":"gemini-3.5-transcribe","input":[{"type":"audio","data":"YXVkaW8=","mime_type":"audio/wav"}]},{"model":"gemini-3.5-transcribe","input":[{"type":"audio","data":"YXVkaW8=","mime_type":"audio/wav"}]},{"model":"gemini-3.5-transcribe","input":[{"type":"audio","data":"YXVkaW8=","mime_type":"audio/wav"}],"generation_config":{"transcription_config":{"mode":{"type":"verbatim","timestamp_granularities":["word"],"diarization_mode":"speaker"}}}}]"#
)]
#[case::language_plus_word_mode(
    "language-plus-word-mode.yaml",
    r#"[{"model":"gemini-3.5-transcribe","input":[{"type":"audio","data":"YXVkaW8=","mime_type":"audio/wav"}],"generation_config":{"transcription_config":{"language_codes":["en-US"],"mode":{"type":"verbatim","timestamp_granularities":["word"],"diarization_mode":"speaker"}}}},{"model":"gemini-3.5-transcribe","input":[{"type":"audio","data":"YXVkaW8=","mime_type":"audio/wav"}],"generation_config":{"transcription_config":{"language_codes":["pt-BR"]}}},{"model":"gemini-3.5-transcribe","input":[{"type":"audio","data":"YXVkaW8=","mime_type":"audio/wav"}],"generation_config":{"transcription_config":{"language_codes":["pt-BR"],"mode":{"type":"verbatim","timestamp_granularities":["word"],"diarization_mode":"speaker"}}}}]"#
)]
#[case::revision_override(
    "revision-override.yaml",
    r#"[{"Content-Type":"application/json","x-goog-api-key":"real-key","Api-Revision":"2026-05-20","X-Caller":"kept"},{"raised":401},{"Content-Type":"application/json","x-goog-api-key":"env-key","Api-Revision":"2026-05-20"}]"#
)]
#[case::incomplete_interaction_error(
    "incomplete-interaction-error.yaml",
    r#"[{"raised":200},{"raised":500},{"text":"","task":"transcribe"},{"raised":201},{"raised":null}]"#
)]
#[case::empty_text_spacing(
    "empty-text-spacing.yaml",
    r#"[{"text":"hello  world","task":"transcribe"},{"text":"","task":"transcribe"},{"text":" ","task":"transcribe"}]"#
)]
#[case::last_word_missing_end(
    "last-word-missing-end.yaml",
    r#"[{"text":"a b","task":"transcribe","words":[{"word":"a","end":5.0},{"word":"b"}]},{"text":"a b","task":"transcribe","words":[{"word":"a","end":5.0},{"word":"b","end":2.0}],"duration":2.0},{"text":"a b c","task":"transcribe","words":[{"word":"a","end":5.0},{"word":"b","end":2.0},{"word":"c","start":1.0}]},{"text":"plain","task":"transcribe"}]"#
)]
#[case::audio_case(
    "audio-case.yaml",
    r#"[{"model":"gemini-3.5-transcribe","input":[{"type":"audio","data":"aMOpbGxvIHfDtnJsZA==","mime_type":"audio/wav"}]},{"text":"","task":"transcribe","usage":{"type":"tokens","input_tokens":2,"output_tokens":8,"total_tokens":10,"input_token_details":{"audio_tokens":5,"text_tokens":-3}}},{"text":"w","task":"transcribe","words":[{"word":"w","start":1.0,"speaker":""}]}]"#
)]
fn the_native_helper_answers_like_litellm(#[case] file: &str, #[case] litellm: &str) {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/studio-transcription")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let answer = super::Op::decode_and_run(&case.call.input).unwrap();
    let expected: serde_json::Value = serde_json::from_str(litellm).unwrap();
    assert_eq!(answer, expected);
}
