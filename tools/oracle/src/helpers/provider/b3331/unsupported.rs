use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_provider::api::video_contracts::config::BaseVideoConfig as _;
use betterlite_provider::api::video_contracts::records::{
    Headers, Params, VideoEditInput, VideoExtensionInput, VideoLogging, VideoReply,
    VideoRequestContext,
};
use serde::Deserialize;
use serde_json::json;

use super::subject::{Subject, SubjectError};
use super::{API_BASE, response};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RequestOperation {
    CreateCharacter,
    GetCharacter,
    Edit,
    Extension,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseOperation {
    CreateCharacter,
    GetCharacter,
    Edit,
    Extension,
}

const EDIT_INPUT: VideoEditInput<'static> = VideoEditInput {
    prompt: "prompt",
    video_id: "video_1",
    video_file: None,
    extra_body: None,
    prefetched_source_data: None,
};

const EXTENSION_INPUT: VideoExtensionInput<'static> = VideoExtensionInput {
    prompt: "prompt",
    video_id: "video_1",
    seconds: "4",
    extra_body: None,
};

pub(super) fn unsupported_request(operation: RequestOperation) -> serde_json::Value {
    let params = Params::new();
    let headers = Headers::new();
    let context = VideoRequestContext {
        api_base: API_BASE,
        litellm_params: &params,
        headers: &headers,
    };
    let outcome = match operation {
        RequestOperation::CreateCharacter => Subject
            .transform_video_create_character_request(
                "name",
                &SuppliedMedia::Bytes(Vec::new()),
                context,
            )
            .map(drop),
        RequestOperation::GetCharacter => Subject
            .transform_video_get_character_request("char_1", context)
            .map(drop),
        RequestOperation::Edit => Subject
            .transform_video_edit_request(EDIT_INPUT, context)
            .map(drop),
        RequestOperation::Extension => Subject
            .transform_video_extension_request(EXTENSION_INPUT, context)
            .map(drop),
    };
    raised(outcome)
}

pub(super) fn unsupported_response(operation: ResponseOperation) -> serde_json::Value {
    let raw = response("");
    let mut logging = VideoLogging::default();
    match operation {
        ResponseOperation::CreateCharacter => {
            let outcome = Subject.transform_video_create_character_response(&raw, &mut logging);
            raised(outcome.map(drop))
        }
        ResponseOperation::GetCharacter => {
            let outcome = Subject.transform_video_get_character_response(&raw, &mut logging);
            raised(outcome.map(drop))
        }
        ResponseOperation::Edit => raised(
            Subject
                .transform_video_edit_response(
                    VideoReply {
                        raw_response: &raw,
                        logging_obj: &mut logging,
                        custom_llm_provider: None,
                    },
                    None,
                )
                .map(drop),
        ),
        ResponseOperation::Extension => raised(
            Subject
                .transform_video_extension_response(VideoReply {
                    raw_response: &raw,
                    logging_obj: &mut logging,
                    custom_llm_provider: None,
                })
                .map(drop),
        ),
    }
}

fn raised(outcome: Result<(), SubjectError>) -> serde_json::Value {
    match outcome {
        Ok(()) => json!({"raised": false, "message": ""}),
        Err(error) => json!({"raised": true, "message": error.to_string()}),
    }
}
