#![cfg(test)]

use betterlite_core::api::video_records::{CharacterObject, VideoObject};
use betterlite_models::decisions::Json;

use crate::api::video_contracts::config::BaseVideoConfig as _;
use crate::api::video_contracts::records::{Params, VideoLogging, VideoReply};

use super::fixture::{FixtureError, Parts, Subject, raw};

#[test]
fn types_create_request_takes_the_mapped_params() {
    let parts = Parts::default();
    let mut mapped = Params::new();
    mapped.insert("seconds".to_owned(), Json::Text("8".to_owned()));
    let (data, files, url) = Subject
        .transform_video_create_request("veo", "a cat", &mapped, parts.context())
        .unwrap();
    assert_eq!(data.get("seconds"), Some(&Json::Text("8".to_owned())));
    assert_eq!(data.get("model"), Some(&Json::Text("veo".to_owned())));
    assert_eq!(data.get("prompt"), Some(&Json::Text("a cat".to_owned())));
    assert!(files.is_empty());
    assert_eq!(url, "https://mock/videos");
}

#[test]
fn types_responses_return_the_b3330_video_record() {
    let response = raw(200, "video_1");
    let mut logging = VideoLogging::default();
    let reply = VideoReply {
        raw_response: &response,
        logging_obj: &mut logging,
        custom_llm_provider: None,
    };
    let video: VideoObject = Subject
        .transform_video_create_response("veo", reply, None)
        .unwrap();
    assert_eq!(video.id, "video_1");
    assert_eq!(video.status, "queued");
}

#[test]
fn types_character_transforms_return_the_b3330_character_record() {
    let mut logging = VideoLogging::default();
    let created: Result<CharacterObject, FixtureError> =
        Subject.transform_video_create_character_response(&raw(200, ""), &mut logging);
    let fetched: Result<CharacterObject, FixtureError> =
        Subject.transform_video_get_character_response(&raw(200, ""), &mut logging);
    assert_eq!(
        created.unwrap_err().to_string(),
        "video create character is not supported for this provider"
    );
    assert_eq!(
        fetched.unwrap_err().to_string(),
        "video get character is not supported for this provider"
    );
}
