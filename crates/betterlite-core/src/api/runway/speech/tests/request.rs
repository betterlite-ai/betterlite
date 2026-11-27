use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::json;

use super::super::mapping::VOICE_KEY;
use super::super::request::speech_request;
use super::support::{request_headers, text_params};

#[test]
fn a_falsey_model_falls_back_and_a_missing_voice_is_bernard() {
    let data = speech_request("", "hello", &Object::new(), &IndexMap::new());
    let body = data.dict_body.unwrap();
    assert_eq!(body.get("model"), Some(&json!("eleven_multilingual_v2")));
    assert_eq!(body.get("promptText"), Some(&json!("hello")));
    assert_eq!(
        body.get("voice"),
        Some(&json!({"type": "runway-preset", "presetId": "Bernard"}))
    );
    assert_eq!(data.ssml_body, None);
}

#[test]
fn a_given_model_is_the_body_model() {
    let body = speech_request("runway-tts", "hi", &Object::new(), &IndexMap::new())
        .dict_body
        .unwrap();
    assert_eq!(body.get("model"), Some(&json!("runway-tts")));
}

#[test]
fn a_null_mapped_voice_chooses_bernard() {
    let mut params = Object::new();
    params.insert(VOICE_KEY.to_owned(), json!(null));
    let body = speech_request("m", "i", &params, &IndexMap::new())
        .dict_body
        .unwrap();
    assert_eq!(
        body.get("voice"),
        Some(&json!({"type": "runway-preset", "presetId": "Bernard"}))
    );
}

#[test]
fn an_empty_mapped_voice_stays_the_body_voice() {
    let mut params = Object::new();
    params.insert(VOICE_KEY.to_owned(), json!({}));
    let body = speech_request("m", "i", &params, &IndexMap::new())
        .dict_body
        .unwrap();
    assert_eq!(body.get("voice"), Some(&json!({})));
}

#[test]
fn a_selected_mapping_voice_is_the_body_voice() {
    let mut params = Object::new();
    params.insert(
        VOICE_KEY.to_owned(),
        json!({"type": "runway-preset", "presetId": "Maya"}),
    );
    let body = speech_request("m", "i", &params, &IndexMap::new())
        .dict_body
        .unwrap();
    assert_eq!(
        body.get("voice"),
        Some(&json!({"type": "runway-preset", "presetId": "Maya"}))
    );
    assert!(!body.contains_key(VOICE_KEY));
}

#[test]
fn extra_options_are_kept_without_overwriting_the_body() {
    let params = text_params(&[
        ("speed", "2"),
        ("seed", "5"),
        ("model", "other"),
        ("promptText", "other"),
        ("voice", "other"),
    ]);
    let body = speech_request("eleven_multilingual_v2", "hello", &params, &IndexMap::new())
        .dict_body
        .unwrap();
    assert_eq!(body.get("speed"), Some(&json!("2")));
    assert_eq!(body.get("seed"), Some(&json!("5")));
    assert_eq!(body.get("model"), Some(&json!("eleven_multilingual_v2")));
    assert_eq!(body.get("promptText"), Some(&json!("hello")));
    assert_eq!(
        body.get("voice"),
        Some(&json!({"type": "runway-preset", "presetId": "Bernard"}))
    );
    assert!(!body.contains_key(VOICE_KEY));
}

#[test]
fn the_request_carries_its_headers() {
    let given = request_headers(&[
        ("Authorization", "Bearer sk-test"),
        ("Content-Type", "application/json"),
    ]);
    let data = speech_request("m", "i", &Object::new(), &given);
    assert_eq!(data.headers, Some(given.clone()));
    assert_eq!(data.dict_body.unwrap().len(), 3);
}
