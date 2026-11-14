#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::Helper as _;

use super::Op;
use super::ops::Input;

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut sources: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("studio_chat_input"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    sources.sort();
    sources
}

#[test]
fn replays_the_studio_chat_input_seeds() {
    let seeds = seed_sources();
    assert!(
        !seeds.is_empty(),
        "the corpus holds a studio_chat_input seed"
    );
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("seed should parse");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "studio_chat_input"
            && Op::check(&case.call.input).is_ok()
    }));
}

fn input(call: &Value) -> Input {
    serde_json::from_value(call.clone()).expect("input should decode")
}

#[test]
fn the_adapter_lists_the_studio_capabilities() {
    let input = input(&json!({
        "reasoning_models": ["gemini-2.5-pro"],
        "steps": [
            {"op": "params", "model": "gemini-2.5-flash-preview-tts"},
            {"op": "params", "model": "gemini-2.0-flash"},
            {"op": "params", "model": "gemini-2.5-pro"}
        ]
    }));

    let output = Op::run(&input).expect("the adapter lists the parameters");

    let results = output["results"].as_array().expect("results are a list");
    let tts = results[0]["params"].as_array().expect("params are a list");
    let plain = results[1]["params"].as_array().expect("params are a list");
    let reasoning = results[2]["params"].as_array().expect("params are a list");
    assert_eq!(tts.len(), 21);
    assert_eq!(tts.last(), Some(&json!("audio")));
    assert_eq!(plain.len(), 20);
    assert!(!plain.contains(&json!("reasoning_effort")));
    assert_eq!(
        reasoning.get(20..),
        Some([json!("reasoning_effort"), json!("thinking")].as_slice())
    );
}

#[test]
fn the_adapter_converts_safe_messages_and_reports_the_rejection() {
    let files_uri = "https://generativelanguage.googleapis.com/v1beta/files/abc123";
    let input = input(&json!({
        "steps": [
            {
                "op": "messages",
                "model": "gemini-2.5-flash",
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": "read"},
                    {"type": "file", "file": {"file_id": files_uri}}
                ]}]
            },
            {
                "op": "messages",
                "model": "gemini-2.5-flash",
                "messages": [{"role": "user", "content": [{"type": "file"}]}]
            }
        ]
    }));

    let output = Op::run(&input).expect("the adapter answers every step");

    assert_eq!(
        output["results"][0]["contents"][0]["parts"][1]["file_data"]["file_uri"],
        json!(files_uri)
    );
    assert_eq!(
        output["results"][1]["error"],
        json!({"status": 400, "type": null, "param": null, "code": "400"})
    );
}
