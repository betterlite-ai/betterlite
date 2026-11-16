use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("valid json")
}

pub(super) fn options(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("a json object")
}

pub(super) fn encoded_video(video_id: &str) -> String {
    encode_video_id_with_provider(video_id, "openai", Some("sora-2"))
}

pub(super) fn keys(body: &IndexMap<String, Json>) -> Vec<&str> {
    body.keys().map(String::as_str).collect()
}
