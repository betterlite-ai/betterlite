#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;

use crate::api::video_records::dict_access::DictAccess;
use crate::api::video_records::{DecodedVideoId, VideoObject};

use super::fixture::{full_video, object, video};

#[test]
fn json_carries_the_declared_fields_in_order() {
    let dumped = full_video().json();
    let names: Vec<&str> = dumped.keys().map(String::as_str).collect();
    assert_eq!(names, VideoObject::FIELDS.to_vec());
}

#[test]
fn json_keeps_null_for_unset_optional_fields() {
    let dumped = video("video_123").json();
    for field in [
        "created_at",
        "completed_at",
        "expires_at",
        "error",
        "progress",
        "remixed_from_video_id",
        "seconds",
        "size",
        "model",
        "usage",
    ] {
        assert_eq!(dumped.get(field), Some(&Json::Null), "{field}");
    }
}

#[test]
fn json_leaves_out_the_hidden_params() {
    let video = full_video();
    let dumped = video.json();
    assert!(!dumped.contains_key("_hidden_params"));
    assert_eq!(dumped.len(), VideoObject::FIELDS.len());
}

#[test]
fn json_matches_the_direct_fields() {
    let video = full_video();
    let dumped = video.json();
    assert_eq!(dumped.get("id"), video.get("id").as_ref());
    assert_eq!(dumped.get("usage"), video.get("usage").as_ref());
}

#[test]
fn json_round_trips_through_serde() {
    let mut video = full_video();
    video.hidden_params = IndexMap::new();
    let dumped = Json::deserialize(serde_json::to_value(&video).unwrap()).unwrap();
    assert_eq!(dumped, Json::Object(video.json()));
    let decoded: VideoObject =
        serde_json::from_value(serde_json::to_value(&video).unwrap()).unwrap();
    assert_eq!(decoded, video);
}

#[test]
fn decoded_video_id_keeps_none_fields_through_serde() {
    let decoded = DecodedVideoId::deserialize(serde_json::json!({"video_id": "video_1"})).unwrap();
    assert_eq!(decoded.custom_llm_provider, None);
    assert_eq!(decoded.model_id, None);
    let dumped = Json::deserialize(serde_json::to_value(&decoded).unwrap()).unwrap();
    assert_eq!(
        dumped,
        Json::Object(object(&[
            ("custom_llm_provider", Json::Null),
            ("model_id", Json::Null),
            ("video_id", Json::Text("video_1".to_owned())),
        ]))
    );
}

#[test]
fn decoded_video_id_keeps_provider_and_model() {
    let decoded: DecodedVideoId = serde_json::from_str(
        r#"{"custom_llm_provider": "openai", "model_id": "sora-2", "video_id": "video_1"}"#,
    )
    .unwrap();
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(decoded.model_id.as_deref(), Some("sora-2"));
    assert_eq!(decoded.video_id, "video_1");
}

#[test]
fn character_json_keeps_the_object_literal() {
    let character = super::fixture::character("char_123");
    assert_eq!(
        character.json().get("object"),
        Some(&Json::Text("character".to_owned()))
    );
}

#[test]
fn video_json_keeps_the_object_literal() {
    assert_eq!(
        video("video_123").json().get("object"),
        Some(&Json::Text("video".to_owned()))
    );
}

#[test]
fn video_json_carries_numbers() {
    let dumped = full_video().json();
    assert_eq!(
        dumped.get("progress"),
        Some(&Json::Number(Number::from(100_i64)))
    );
}

#[test]
fn video_object_defaults_missing_optional_fields() {
    let decoded: VideoObject = serde_json::from_str(
        r#"{"id": "video_1", "object": "video", "status": "queued", "seconds": null}"#,
    )
    .unwrap();
    assert_eq!(decoded.created_at, None);
    assert_eq!(decoded.progress, None);
    assert_eq!(decoded.seconds, None);
    assert!(decoded.hidden_params.is_empty());
    assert_eq!(decoded.get("model"), Some(Json::Null));
    assert!(
        serde_json::from_str::<VideoObject>(r#"{"id": "video_1", "object": "video"}"#).is_err()
    );
    assert!(
        serde_json::from_str::<VideoObject>(r#"{"id": "video_1", "status": "queued"}"#).is_err()
    );
}
