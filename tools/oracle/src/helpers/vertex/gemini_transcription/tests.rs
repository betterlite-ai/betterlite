#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_vertex_gemini_transcription() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_gemini_transcription"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "vertex_gemini_transcription"
    }));
}

// the expected answers are LiteLLM's own, captured from the Python helper on the same case
#[rstest]
#[case::global_default(
    "global-default.yaml",
    r#"["https://aiplatform.googleapis.com/v1/projects/my-project/locations/global/publishers/google/models/gemini-2.0-flash:generateContent","https://aiplatform.googleapis.com/v1/projects/my-project/locations/global/publishers/google/models/gemini-2.0-flash:generateContent","https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/gemini-2.0-flash:generateContent","https://aiplatform.us.rep.googleapis.com/v1/projects/my-project/locations/us/publishers/google/models/gemini-2.0-flash:generateContent","https://proxy.example/v1/projects/my-project/locations/global/publishers/google/models/vertex_ai/model:generateContent"]"#
)]
#[case::language_config_empty(
    "language-config-empty.yaml",
    r#"[{"contents":[{"parts":[{"inlineData":{"data":"YXVkaW8=","mimeType":"audio/wav"}}],"role":"user"}],"generationConfig":{"audioTranscriptionConfig":{}}},{"contents":[{"parts":[{"inlineData":{"data":"YXVkaW8=","mimeType":"audio/wav"}}],"role":"user"}],"generationConfig":{"audioTranscriptionConfig":{}}}]"#
)]
#[case::inline_audio_only(
    "inline-audio-only.yaml",
    r#"[{"contents":[{"parts":[{"inlineData":{"data":"YXVkaW8=","mimeType":"audio/wav"}}],"role":"user"}],"generationConfig":{"audioTranscriptionConfig":{"languageCodes":["en-US"]}}},{"contents":[{"parts":[{"inlineData":{"data":"aGVsbG8gd29ybGQ=","mimeType":"audio/wav"}}],"role":"user"}],"generationConfig":{"audioTranscriptionConfig":{"languageCodes":["de-DE"]}}}]"#
)]
#[case::all_candidate_text(
    "all-candidate-text.yaml",
    r#"[{"task":"transcribe","text":"hello there world"},{"task":"transcribe","text":"    a b"}]"#
)]
#[case::audio_modality_case(
    "audio-modality-case.yaml",
    r#"[{"task":"transcribe","text":"","usage":{"input_token_details":{"audio_tokens":6,"text_tokens":4},"input_tokens":10,"output_tokens":4,"total_tokens":14,"type":"tokens"}},{"task":"transcribe","text":"","usage":{"input_token_details":{"audio_tokens":6,"text_tokens":4},"input_tokens":10,"output_tokens":0,"total_tokens":0,"type":"tokens"}}]"#
)]
#[case::negative_text_tokens(
    "negative-text-tokens.yaml",
    r#"[{"task":"transcribe","text":"","usage":{"input_token_details":{"audio_tokens":7,"text_tokens":-5},"input_tokens":2,"output_tokens":5,"total_tokens":9,"type":"tokens"}},{"task":"transcribe","text":"","usage":{"input_token_details":{"audio_tokens":6,"text_tokens":-10},"input_tokens":-4,"output_tokens":-1,"total_tokens":-5,"type":"tokens"}}]"#
)]
#[case::absent_usage(
    "absent-usage.yaml",
    r#"[{"task":"transcribe","text":""},{"task":"transcribe","text":""},{"task":"transcribe","text":"","usage":{"input_token_details":{"audio_tokens":0,"text_tokens":0},"input_tokens":0,"output_tokens":0,"total_tokens":0,"type":"tokens"}}]"#
)]
fn the_native_helper_answers_like_litellm(#[case] file: &str, #[case] litellm: &str) {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/vertex-gemini-transcription")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let answer = super::Op::decode_and_run(&case.call.input).unwrap();
    let expected: serde_json::Value = serde_json::from_str(litellm).unwrap();
    assert_eq!(answer, expected);
}
