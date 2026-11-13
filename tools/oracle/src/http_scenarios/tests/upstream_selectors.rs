#![cfg(test)]

use serde_json::{Map, Value};

use super::support::{capture_with_provider, provider_request, step_with_compare};
use crate::case::Ignore;
use crate::http_scenarios::compare::compare_transcripts;
use crate::http_scenarios::model::{Step, StepCompare};
use crate::http_scenarios::run::StepCapture;

fn body(text: &str) -> Value {
    let mut body = Map::new();
    body.insert("echo".to_owned(), Value::String(text.to_owned()));
    Value::Object(body)
}

fn captured(text: &str) -> StepCapture {
    let request = provider_request("POST", "/v1/echo", body(text));
    capture_with_provider(Value::Null, vec![request])
}

fn step(ignored: Option<&str>) -> Step {
    let compare = StepCompare {
        headers: Vec::new(),
        headers_present: Vec::new(),
        upstream_headers: Vec::new(),
        ignore: ignored
            .map(|pointer| Ignore {
                pointer: pointer.to_owned(),
                why: "the provider body differs on every run".to_owned(),
            })
            .into_iter()
            .collect(),
    };
    step_with_compare(compare)
}

#[test]
fn step_selector_uses_step_captures() {
    let first = vec![captured("first"), captured("first")];
    let second = vec![captured("changed"), captured("changed")];

    let unselected = vec![step(None), step(None)];
    let detected = compare_transcripts(&first, &second, &unselected).unwrap_err();
    assert_eq!(detected.pointer, "/steps/0/provider/0/body");

    let selected = vec![step(Some("/upstream/0/body")), step(None)];
    let skipped = compare_transcripts(&first, &second, &selected).unwrap_err();
    assert_eq!(skipped.pointer, "/steps/1/provider/0/body");
    assert_eq!(skipped.message, "provider request bodies differ");
}
