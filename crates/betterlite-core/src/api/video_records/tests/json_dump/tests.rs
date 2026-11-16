#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::api::video_records::dict_access::DictAccess as _;
use crate::api::video_records::{DecodedVideoId, VideoObject};

use super::fixture::{full_video, object, video};

#[test]
fn json_carries_the_declared_fields_in_order() {
    let dumped = full_video().json();
    let names: Vec<&str> = dumped.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "id",
            "object",
            "status",
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
        ]
    );
}

#[rstest]
#[case::created_at("created_at")]
#[case::completed_at("completed_at")]
#[case::expires_at("expires_at")]
#[case::error("error")]
#[case::progress("progress")]
#[case::remixed_from_video_id("remixed_from_video_id")]
#[case::seconds("seconds")]
#[case::size("size")]
#[case::model("model")]
#[case::usage("usage")]
fn json_keeps_null_for_unset_optional_fields(#[case] field: &str) {
    let dumped = video("video_123").json();
    assert_eq!(dumped.get(field), Some(&Json::Null), "{field}");
}

#[test]
fn json_leaves_out_the_hidden_params() {
    let video = full_video();
    let dumped = video.json();
    assert!(!dumped.contains_key("_hidden_params"));
    assert_eq!(dumped.len(), 13);
}

#[test]
fn json_matches_the_direct_fields() {
    let video = full_video();
    let dumped = video.json();
    assert_eq!(dumped.get("id"), Some(&Json::Text(video.id.clone())));
    assert_eq!(
        dumped.get("usage"),
        Some(&Json::Object(video.usage.unwrap()))
    );
}

#[test]
fn json_round_trips_through_serde() {
    let mut video = full_video();
    video.hidden_params = IndexMap::new();
    let dumped: Json = serde_json::from_value(serde_json::to_value(&video).unwrap()).unwrap();
    assert_eq!(dumped, Json::Object(video.json()));
    let decoded: VideoObject =
        serde_json::from_value(serde_json::to_value(&video).unwrap()).unwrap();
    assert_eq!(decoded, video);
}

#[test]
fn decoded_video_id_keeps_none_fields_through_serde() {
    let decoded: DecodedVideoId = serde_json::from_str(r#"{"video_id": "video_1"}"#).unwrap();
    assert_eq!(decoded.custom_llm_provider, None);
    assert_eq!(decoded.model_id, None);
    let dumped: Json = serde_json::from_value(serde_json::to_value(&decoded).unwrap()).unwrap();
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
    let missing_status =
        serde_json::from_str::<VideoObject>(r#"{"id": "video_1", "object": "video"}"#).unwrap_err();
    let missing_object =
        serde_json::from_str::<VideoObject>(r#"{"id": "video_1", "status": "queued"}"#)
            .unwrap_err();
    assert!(
        missing_status
            .to_string()
            .contains("missing field `status`")
    );
    assert!(
        missing_object
            .to_string()
            .contains("missing field `object`")
    );
}

#[test]
fn json_new_matches_the_decoded_defaults() {
    let built = VideoObject::new("video_1".to_owned(), "queued".to_owned());
    assert_eq!(built, video("video_1"));
}
