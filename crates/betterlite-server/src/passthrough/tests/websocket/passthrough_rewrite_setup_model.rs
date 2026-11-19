use rstest::rstest;
use serde_json::json;

use super::rewriter;
use crate::passthrough::websocket::rewrite_live_setup_model;

#[rstest]
#[case::empty("")]
#[case::plain_text("not json")]
#[case::array("[1,2]")]
#[case::object_without_setup(r#"{"model":"m"}"#)]
#[case::setup_not_an_object(r#"{"setup":"m"}"#)]
#[case::setup_without_a_model(r#"{"setup":{"temperature":1}}"#)]
#[case::setup_model_not_a_string(r#"{"setup":{"model":3}}"#)]
fn frames_that_are_not_setup_frames_stay_identical(#[case] text: &str) {
    assert_eq!(rewrite_live_setup_model(text, None), text);
    assert_eq!(rewrite_live_setup_model(text, Some(&rewriter)), text);
}

#[test]
fn an_unchanged_rewrite_returns_the_input_bytes() {
    let same = |model: &str| model.to_owned();
    let input = r#"{ "setup" : { "model" : "alias" } }"#;
    assert_eq!(rewrite_live_setup_model(input, Some(&same)), input);
}

#[test]
fn a_setup_frame_gets_the_rewritten_model() {
    let input = r#"{"setup":{"model":"alias","generationConfig":{"n":1}},"extra":true}"#;
    let output = rewrite_live_setup_model(input, Some(&rewriter));
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(
        value,
        json!({
            "setup": {
                "model": "projects/p/locations/l/models/alias",
                "generationConfig": {"n": 1}
            },
            "extra": true
        })
    );
}
