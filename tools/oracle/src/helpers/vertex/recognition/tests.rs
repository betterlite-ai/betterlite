#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_vertex_recognition() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_recognition"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "vertex_recognition"
    }));
}

// the expected answers are LiteLLM's own, captured from the Python helper on the same case
#[rstest]
#[case::default_us_auto(
    "default-us-auto.yaml",
    r#"[{"language":"en"},"https://us-speech.googleapis.com/v2/projects/my-project/locations/us/recognizers/_:recognize",{"config":{"model":"gemini-2.0-flash","languageCodes":["en-US"],"features":{"enableAutomaticPunctuation":true},"autoDecodingConfig":{}},"content":"YXVkaW8="}]"#
)]
#[case::global_host(
    "global-host.yaml",
    r#"["https://speech.googleapis.com/v2/projects/my-project/locations/global/recognizers/_:recognize","https://proxy.example/v2/projects/my-project/locations/global/recognizers/_:recognize","https://europe-west1-speech.googleapis.com/v2/projects/my-project/locations/europe-west1/recognizers/_:recognize"]"#
)]
#[case::unsafe_project(
    "unsafe-project.yaml",
    r#"[{"raised":400},{"raised":400},{"raised":400},{"raised":400}]"#
)]
#[case::drop_subtitles(
    "drop-subtitles.yaml",
    r#"[{},{},{"raised":400},{"raised":400},{"language":"de","response_format":"text"}]"#
)]
#[case::first_alternatives(
    "first-alternatives.yaml",
    r#"[{"text":"hello world","task":"transcribe","language":"en"},{"text":"","task":"transcribe"}]"#
)]
#[case::first_detected_language(
    "first-detected-language.yaml",
    r#"[{"text":"hi there now","task":"transcribe","language":"de"},{"text":"hi","task":"transcribe"},{"text":"hi there","task":"transcribe","language":"fr"}]"#
)]
#[case::malformed_duration(
    "malformed-duration.yaml",
    r#"[{"text":"","task":"transcribe","duration":2.5},{"text":"","task":"transcribe","duration":2.0},{"text":"","task":"transcribe"},{"text":"","task":"transcribe"},{"text":"","task":"transcribe"},{"text":"","task":"transcribe"},{"text":"","task":"transcribe"}]"#
)]
fn the_native_helper_answers_like_litellm(#[case] file: &str, #[case] litellm: &str) {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/vertex-recognition")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let answer = super::Op::decode_and_run(&case.call.input).unwrap();
    let expected: serde_json::Value = serde_json::from_str(litellm).unwrap();
    assert_eq!(answer, expected);
}
