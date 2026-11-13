#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

fn run_case(file: &str) -> serde_json::Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/xai-stt")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_xai_stt() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("xai_stt"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "xai_stt"
    }));
}

#[test]
fn oracle_request_flattens_extra_body_and_spells_booleans() {
    let result = run_case("flatten-and-boolean-form.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[0]["supported"], serde_json::json!(["language"]));
    assert_eq!(
        calls[0]["mapped"],
        serde_json::json!({"language": "en", "response_format": "json"})
    );
    assert_eq!(calls[0]["url"], "https://api.x.ai/v1/stt");
    assert_eq!(calls[0]["authorization"], "Bearer sk-key");
    assert_eq!(
        calls[0]["form"],
        serde_json::json!([
            ["model", "grok-stt"],
            ["custom", "x"],
            ["diarize", "true"],
            ["language", "en"],
            ["temperature", "0.5"],
            ["prompt", "hi"],
            ["timestamp_granularities", ["word", "segment"]],
        ])
    );
    assert_eq!(calls[0]["file"]["filename"], "clip.wav");
    assert_eq!(calls[0]["file"]["content_type"], "audio/wav");
    assert_eq!(
        calls[0]["file"]["content"],
        serde_json::json!([97_i64, 117_i64, 100_i64, 105_i64, 111_i64])
    );
}

#[test]
fn oracle_request_keeps_python_spelling_and_needs_a_key() {
    let result = run_case("flatten-and-boolean-form.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(calls[1]["url"], "https://proxy.example/v1/stt");
    assert_eq!(
        calls[1]["form"],
        serde_json::json!([
            ["model", "m"],
            ["f", "1.5"],
            ["flag", "false"],
            ["items", ["True", "False", "1"]],
            ["n", "3"],
        ])
    );
    assert_eq!(calls[1]["file"]["filename"], "audio.wav");
    assert_eq!(calls[2]["raised"], "key");
    assert_eq!(calls[2]["url"], "https://proxy.example/v2/v1/stt");
    assert_eq!(calls[2]["form"], serde_json::json!([]));
    assert_eq!(calls[2]["file"], serde_json::Value::Null);
}

#[test]
fn oracle_reply_keeps_a_zero_speaker_and_the_duration() {
    let result = run_case("speaker-zero-and-duration.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0],
        serde_json::json!({
            "text": "hello",
            "task": "transcribe",
            "language": "en",
            "duration": 2.5_f64,
            "words": [
                {"word": "he", "start": 0.0_f64, "end": 1.0_f64, "speaker": 0_i64},
                {"word": "llo", "start": 1.0_f64, "end": 2.0_f64},
            ],
        })
    );
    assert_eq!(
        calls[1],
        serde_json::json!({
            "text": "hi",
            "task": "transcribe",
            "language": "de",
            "words": [],
        })
    );
}

#[test]
fn oracle_reply_defaults_and_malformed_success() {
    let result = run_case("defaults-and-malformed-success.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0],
        serde_json::json!({"text": "", "task": "transcribe", "language": "unknown"})
    );
    assert_eq!(calls[1]["error"]["status"], 422_i64);
    assert_eq!(calls[2]["error"]["status"], 201_i64);
    assert_eq!(calls[3]["error"]["status"], 200_i64);
}
