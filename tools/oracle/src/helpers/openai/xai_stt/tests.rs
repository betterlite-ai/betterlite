#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};

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
        validate_helper(Path::new("<helper>"), &case).is_ok() && case.call.operation == "xai_stt"
    }));
}

// the expected answers are LiteLLM's own, captured from the Python helper on the same case
#[rstest]
#[case::defaults_and_malformed_success("defaults-and-malformed-success.yaml", r#"[{"text":"","task":"transcribe","language":"unknown"},{"raised":422},{"raised":201},{"raised":200}]"#)]
#[case::flatten_and_boolean_form("flatten-and-boolean-form.yaml", r#"[{"supported":["language"],"mapped":{"response_format":"json","language":"en"}},"https://api.x.ai/v1/stt","https://proxy.example/v1/stt","https://proxy.example/v2/v1/stt","Bearer sk-key",{"raised":null},{"form":[["model","grok-stt"],["custom","x"],["diarize","true"],["language","en"],["temperature","0.5"],["prompt","hi"],["timestamp_granularities",["word","segment"]]],"file":["clip.wav","audio/wav",[97,117,100,105,111]]},{"form":[["model","m"],["f","1.5"],["flag","false"],["items",["True","False","1"]],["n","3"]],"file":["audio.wav","audio/wav",[114,97,119]]},{"form":[["model","m"]],"file":["sound.flac","audio/flac",[97,98,99]]}]"#)]
#[case::speaker_zero_and_duration("speaker-zero-and-duration.yaml", r#"[{"text":"hello","task":"transcribe","language":"en","duration":2.5,"words":[{"word":"he","start":0.0,"end":1.0,"speaker":0},{"word":"llo","start":1.0,"end":2.0}]},{"text":"hi","task":"transcribe","language":"de","words":[]}]"#)]
fn the_native_helper_answers_like_litellm(#[case] file: &str, #[case] litellm: &str) {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/xai-stt")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let answer = super::Op::decode_and_run(&case.call.input).unwrap();
    let expected: serde_json::Value = serde_json::from_str(litellm).unwrap();
    assert_eq!(answer, expected);
}
