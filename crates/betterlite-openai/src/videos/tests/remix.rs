use serde_json::json;

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::videos::VideoOptions;
use crate::videos::remix::{remix_request, remix_response};

use super::support::{API_BASE, MODEL, SLASH_BASE, model_of, options, video_body, video_of};

const REMIX_FIXTURE: &str = r#"{
    "id": "vid-4",
    "object": "video",
    "status": "queued",
    "seconds": "4",
    "usage": {"ignored": true}
}"#;

fn remix_url_case() {
    let encoded = encode_video_id_with_provider("vid/one two", "openai", Some(MODEL));
    let (url, data) =
        remix_request(&encoded, "make it blue", SLASH_BASE, &VideoOptions::new()).unwrap();
    assert_eq!(url, "https://mock.example/v1/videos/vid%2Fone%20two/remix");
    assert_eq!(data, options(&[("prompt", json!("make it blue"))]));
}

fn remix_body_overwrite_case() {
    let encoded = encode_video_id_with_provider("vid-4", "openai", Some(MODEL));
    let extra = options(&[("prompt", json!("overwritten")), ("seconds", json!("4"))]);
    let (_, merged) = remix_request(&encoded, "make it blue", API_BASE, &extra).unwrap();
    assert_eq!(merged, extra);
}

fn remix_reply_case() {
    let remixed = remix_response(REMIX_FIXTURE.as_bytes(), Some("openai")).unwrap();
    assert_eq!(video_of(&remixed.id), "vid-4");
    // the wrap passes no model, which the codec decodes back as an empty string
    assert_eq!(model_of(&remixed.id).as_deref(), Some(""));
    assert_eq!(
        remixed.usage,
        Some(options(&[("duration_seconds", json!(4.0_f64))]))
    );
}

fn remix_native_case() {
    let native = remix_response(&video_body("vid-4", Some(json!("abc"))), None).unwrap();
    assert_eq!(native.id, "vid-4");
    assert!(native.usage.unwrap().is_empty());
    let zero = remix_response(&video_body("vid-4", Some(json!(0_i64))), Some("openai")).unwrap();
    assert!(zero.usage.unwrap().is_empty());
}

pub(super) fn remix_case() {
    remix_url_case();
    remix_body_overwrite_case();
    remix_reply_case();
    remix_native_case();
}
