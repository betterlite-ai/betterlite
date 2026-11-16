#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::video_edits::response::{CharacterKind, VideoKind};
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::video_records::{CharacterObject, VideoObject, VideoRecordError};

pub(super) fn video(id: &str) -> VideoObject {
    VideoObject {
        id: id.to_owned(),
        object: VideoKind::Video,
        status: "queued".to_owned(),
        created_at: None,
        completed_at: None,
        expires_at: None,
        error: None,
        progress: None,
        remixed_from_video_id: None,
        seconds: None,
        size: None,
        model: None,
        usage: None,
        hidden_params: IndexMap::new(),
    }
}

pub(super) fn full_video() -> VideoObject {
    VideoObject {
        id: "video_123".to_owned(),
        object: VideoKind::Video,
        status: "completed".to_owned(),
        created_at: Some(1_700_000_000),
        completed_at: Some(1_700_000_100),
        expires_at: Some(1_700_086_400),
        error: Some(object(&[("code", Json::Text("video_unsafe".to_owned()))])),
        progress: Some(100),
        remixed_from_video_id: Some("video_parent".to_owned()),
        seconds: Some("8".to_owned()),
        size: Some("1280x720".to_owned()),
        model: Some("sora-2".to_owned()),
        usage: Some(object(&[(
            "total_tokens",
            Json::Number(Number::from(64_i64)),
        )])),
        hidden_params: object(&[("routing", Json::Text("openai".to_owned()))]),
    }
}

pub(super) fn character(id: &str) -> CharacterObject {
    CharacterObject {
        id: id.to_owned(),
        object: CharacterKind::Character,
        created_at: 1_700_000_000,
        name: "Ada".to_owned(),
        hidden_params: IndexMap::new(),
    }
}

pub(super) fn object(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut fields = IndexMap::new();
    for (key, value) in pairs {
        fields.insert((*key).to_owned(), value.clone());
    }
    fields
}

pub(super) fn assert_not_text(error: VideoRecordError, expected: &str) {
    match error {
        VideoRecordError::NotText { field } => assert_eq!(field, expected),
        VideoRecordError::UnknownField { object, field } => {
            panic!("expected NotText for {expected}, got UnknownField on {object}.{field}")
        }
    }
}

pub(super) fn assert_unknown_field(
    error: VideoRecordError,
    expected_object: &str,
    expected_field: &str,
) {
    match error {
        VideoRecordError::UnknownField { object, field } => {
            assert_eq!(object, expected_object);
            assert_eq!(field, expected_field);
        }
        VideoRecordError::NotText { field } => {
            panic!(
                "expected UnknownField on {expected_object}.{expected_field}, got NotText for {field}"
            )
        }
    }
}
