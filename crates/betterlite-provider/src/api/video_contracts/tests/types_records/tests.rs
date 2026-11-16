#![cfg(test)]

use betterlite_core::api::video_records::{CharacterObject, VideoObject};

use crate::api::video_contracts::config::BaseVideoConfig as _;
use crate::api::video_contracts::records::{VideoLogging, VideoReply};

use super::fixture::{FixtureError, Parts, Subject, empty_options, raw};

#[test]
fn types_create_request_takes_the_b3330_option_record() {
    let parts = Parts::default();
    let mut options = empty_options();
    options.seconds = Some("8".to_owned());
    let (data, files, url) = Subject
        .transform_video_create_request("veo", "a cat", &options, parts.context())
        .unwrap();
    assert_eq!(data.len(), 2);
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
    assert!(created.is_err());
    assert!(fetched.is_err());
}
