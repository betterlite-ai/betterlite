#![cfg(test)]

use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::api::video_records::dict_access::DictAccess;

use super::fixture::{full_video, object, video};

#[test]
fn get_returns_the_video_identity_fields() {
    let video = full_video();
    assert_eq!(video.get("id"), Some(Json::Text("video_123".to_owned())));
    assert_eq!(video.get("object"), Some(Json::Text("video".to_owned())));
    assert_eq!(
        video.get("status"),
        Some(Json::Text("completed".to_owned()))
    );
}

#[test]
fn get_returns_the_video_timestamps() {
    let video = full_video();
    assert_eq!(
        video.get("created_at"),
        Some(Json::Number(Number::from(1_700_000_000_i64)))
    );
    assert_eq!(
        video.get("completed_at"),
        Some(Json::Number(Number::from(1_700_000_100_i64)))
    );
    assert_eq!(
        video.get("expires_at"),
        Some(Json::Number(Number::from(1_700_086_400_i64)))
    );
    assert_eq!(
        video.get("progress"),
        Some(Json::Number(Number::from(100_i64)))
    );
}

#[test]
fn get_returns_the_video_text_fields() {
    let video = full_video();
    assert_eq!(
        video.get("remixed_from_video_id"),
        Some(Json::Text("video_parent".to_owned()))
    );
    assert_eq!(video.get("seconds"), Some(Json::Text("8".to_owned())));
    assert_eq!(video.get("size"), Some(Json::Text("1280x720".to_owned())));
    assert_eq!(video.get("model"), Some(Json::Text("sora-2".to_owned())));
}

#[test]
fn get_returns_the_video_object_fields() {
    let video = full_video();
    assert_eq!(
        video.get("error"),
        Some(Json::Object(object(&[(
            "code",
            Json::Text("video_unsafe".to_owned())
        )])))
    );
    assert_eq!(
        video.get("usage"),
        Some(Json::Object(object(&[(
            "total_tokens",
            Json::Number(Number::from(64_i64))
        )])))
    );
}

#[test]
fn get_returns_null_for_unset_video_fields() {
    let video = video("video_123");
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
        assert_eq!(video.get(field), Some(Json::Null), "{field}");
    }
}

#[test]
fn get_returns_the_hidden_params() {
    let video = full_video();
    assert_eq!(
        video.get("_hidden_params"),
        Some(Json::Object(object(&[(
            "routing",
            Json::Text("openai".to_owned())
        )])))
    );
}

#[test]
fn get_returns_none_for_an_unknown_field() {
    assert_eq!(full_video().get("duration"), None);
}
