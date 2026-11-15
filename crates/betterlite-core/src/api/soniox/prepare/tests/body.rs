use betterlite_openai::openai_base_config::Object;
use rstest::rstest;
use serde_json::json;

use super::super::body::build_create_body;
use super::super::options::{HandlerOptions, extract_options};

fn params(text: &str) -> Object {
    serde_json::from_str(text).expect("the case params are a JSON object")
}

fn prepared_params(text: &str) -> (Object, HandlerOptions) {
    let mut params = params(text);
    let options = extract_options(&mut params).expect("the params are accepted");
    (params, options)
}

#[test]
fn writes_the_model_and_non_null_options() {
    let (params, options) = prepared_params(
        r#"{"model": "override", "language_hints": ["en"], "client_reference_id": null, "context": {"topic": "a"}}"#,
    );

    let body = build_create_body("stt-async-v4", &params, &options, None);

    assert_eq!(body.get("model"), Some(&json!("override")));
    assert_eq!(body.get("language_hints"), Some(&json!(["en"])));
    let context = body.get("context").map(ToString::to_string);
    assert_eq!(context, Some(String::from(r#"{"topic":"a"}"#)));
    assert!(!body.contains_key("client_reference_id"));
    assert_eq!(body.len(), 3);
}

#[test]
fn writes_only_the_model_for_empty_options() {
    let (params, options) = prepared_params("{}");

    let body = build_create_body("stt-async-v4", &params, &options, None);

    assert_eq!(body.get("model"), Some(&json!("stt-async-v4")));
    assert_eq!(body.len(), 1);
}

#[test]
fn writes_both_references() {
    let (params, options) =
        prepared_params(r#"{"audio_url": "https://a.test/x.wav", "file_id": "file_1"}"#);

    let body = build_create_body("stt-async-v4", &params, &options, Some("file_1"));

    assert_eq!(body.get("audio_url"), Some(&json!("https://a.test/x.wav")));
    assert_eq!(body.get("file_id"), Some(&json!("file_1")));
}

#[test]
fn writes_a_reference_from_the_upload() {
    let (params, options) = prepared_params("{}");

    let body = build_create_body("stt-async-v4", &params, &options, Some("file_uploaded"));

    assert_eq!(body.get("file_id"), Some(&json!("file_uploaded")));
    assert!(!body.contains_key("audio_url"));
}

#[rstest]
#[case::empty_url(r#"{"audio_url": "", "file_id": ""}"#, Some(""))]
#[case::null_references(r#"{"audio_url": null, "file_id": null}"#, None)]
fn omits_empty_references(#[case] input: &str, #[case] selected: Option<&str>) {
    let (params, options) = prepared_params(input);

    let body = build_create_body("stt-async-v4", &params, &options, selected);

    assert!(!body.contains_key("audio_url"));
    assert!(!body.contains_key("file_id"));
    assert_eq!(body.len(), 1);
}
