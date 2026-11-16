#![cfg(test)]

use betterlite_core::api::bfl::edit::media::SuppliedMedia;

use crate::api::video_contracts::config::BaseVideoConfig as _;
use crate::api::video_contracts::records::{
    VideoEditInput, VideoExtensionInput, VideoLogging, VideoReply,
};

use super::fixture::{FixtureError, Parts, Subject, raw};

fn message<T>(result: Result<T, FixtureError>) -> String {
    match result {
        Ok(_) => "supported".to_owned(),
        Err(error) => error.to_string(),
    }
}

#[test]
fn unsupported_create_character_request() {
    let parts = Parts::default();
    let video = SuppliedMedia::Bytes(Vec::new());
    let result = Subject.transform_video_create_character_request("name", &video, parts.context());
    assert_eq!(
        message(result),
        "video create character is not supported for this provider"
    );
}

#[test]
fn unsupported_create_character_response() {
    let mut logging = VideoLogging::default();
    let result = Subject.transform_video_create_character_response(&raw(200, ""), &mut logging);
    assert_eq!(
        message(result),
        "video create character is not supported for this provider"
    );
}

#[test]
fn unsupported_get_character_request() {
    let parts = Parts::default();
    let result = Subject.transform_video_get_character_request("char_1", parts.context());
    assert_eq!(
        message(result),
        "video get character is not supported for this provider"
    );
}

#[test]
fn unsupported_get_character_response() {
    let mut logging = VideoLogging::default();
    let result = Subject.transform_video_get_character_response(&raw(200, ""), &mut logging);
    assert_eq!(
        message(result),
        "video get character is not supported for this provider"
    );
}

#[test]
fn unsupported_edit_request() {
    let parts = Parts::default();
    let input = VideoEditInput {
        prompt: "prompt",
        video_id: "video_1",
        video_file: None,
        extra_body: None,
        prefetched_source_data: None,
    };
    let result = Subject.transform_video_edit_request(input, parts.context());
    assert_eq!(
        message(result),
        "video edit is not supported for this provider"
    );
}

#[test]
fn unsupported_edit_response() {
    let response = raw(200, "");
    let mut logging = VideoLogging::default();
    let reply = VideoReply {
        raw_response: &response,
        logging_obj: &mut logging,
        custom_llm_provider: None,
    };
    let result = Subject.transform_video_edit_response(reply, None);
    assert_eq!(
        message(result),
        "video edit is not supported for this provider"
    );
}

#[test]
fn unsupported_extension_request() {
    let parts = Parts::default();
    let input = VideoExtensionInput {
        prompt: "prompt",
        video_id: "video_1",
        seconds: "4",
        extra_body: None,
    };
    let result = Subject.transform_video_extension_request(input, parts.context());
    assert_eq!(
        message(result),
        "video extension is not supported for this provider"
    );
}

#[test]
fn unsupported_extension_response() {
    let response = raw(200, "");
    let mut logging = VideoLogging::default();
    let reply = VideoReply {
        raw_response: &response,
        logging_obj: &mut logging,
        custom_llm_provider: None,
    };
    let result = Subject.transform_video_extension_response(reply);
    assert_eq!(
        message(result),
        "video extension is not supported for this provider"
    );
}
