#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::{HelperCall, invoke};

fn seeds() -> Vec<(String, HelperCase)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds: Vec<(String, HelperCase)> = fs::read_dir(corpus)
        .expect("the helper corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("audio_subtitles"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed");
            let case: HelperCase = serde_saphyr::from_str(&source).expect("a helper case");
            (path.display().to_string(), case)
        })
        .collect();
    seeds.sort_by(|left, right| left.0.cmp(&right.0));
    seeds
}

#[test]
fn helper_seeds_select_audio_subtitles() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let rejected: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| {
            let decodes = serde_json::from_value::<super::Input>(case.call.input.clone()).is_ok();
            case.call.operation != "audio_subtitles"
                || !decodes
                || validate_helper(Path::new("<helper>"), case).is_err()
        })
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(rejected.is_empty(), "{rejected:?}");
}

#[test]
fn helper_seeds_cover_token_rendering_and_word_timings() {
    let inputs: Vec<super::Input> = seeds()
        .iter()
        .map(|(_, case)| {
            serde_json::from_value(case.call.input.clone()).expect("an audio_subtitles input")
        })
        .collect();
    assert!(inputs.iter().any(|input| {
        input
            .calls
            .iter()
            .any(|call| matches!(call, super::Call::Group { .. }))
    }));
    assert!(inputs.iter().any(|input| {
        input
            .calls
            .iter()
            .any(|call| matches!(call, super::Call::Srt { .. } | super::Call::Vtt { .. }))
    }));
    assert!(inputs.iter().any(|input| {
        input
            .calls
            .iter()
            .any(|call| matches!(call, super::Call::Synthesize { .. }))
    }));
}

#[test]
fn helper_runs_the_production_subtitle_renderer() {
    let call = HelperCall {
        operation: "audio_subtitles".to_owned(),
        input: json!({"calls": [
            {"function": "srt", "tokens": [{"text": "Hell", "start_ms": 0}, {"text": "o", "end_ms": 1000}]}
        ]}),
    };
    let output = invoke(&call);
    assert!(output.ok);
    assert_eq!(
        output.value,
        Some(json!([{"ok": true, "value": "1\n00:00:00,000 --> 00:00:01,000\nHello\n"}]))
    );
}

#[test]
fn helper_runs_the_production_word_timing_transformer() {
    let call = HelperCall {
        operation: "audio_subtitles".to_owned(),
        input: json!({"calls": [
            {"function": "synthesize", "response_format": "srt", "words": [{"word": "Hi", "start": 0, "end": 1}]},
            {"function": "synthesize", "response_format": "json", "words": []}
        ]}),
    };
    let output = invoke(&call);
    assert!(output.ok);
    assert_eq!(
        output.value,
        Some(json!([
            {"ok": true, "value": "1\n00:00:00,000 --> 00:00:01,000\nHi\n"},
            {"ok": true, "value": null}
        ]))
    );
}
