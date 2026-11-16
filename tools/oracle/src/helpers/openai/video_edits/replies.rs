use betterlite_models::decisions::Json;
use betterlite_openai::video_edits::response::{
    VideoReply, decode_character_reply, transform_video_response,
};

use super::CharacterResponse;
use super::support::helper_error;
use betterlite_oracle_helper_api::helper::HelperResult;

pub(super) fn character_reply(reply: &CharacterResponse) -> HelperResult {
    match decode_character_reply(reply.body.as_bytes()) {
        Ok(object) => serde_json::to_value(&object)
            .map_err(|error| helper_error("InvalidInput", error.to_string())),
        Err(error) => Err(helper_error("VideoEditError", error.to_string())),
    }
}

pub(super) fn video_reply_value(
    body: &str,
    provider: Option<&str>,
    request_data: Option<&Json>,
) -> HelperResult {
    let reply = VideoReply {
        body: body.as_bytes(),
        custom_llm_provider: provider,
        request_data,
    };
    match transform_video_response(&reply) {
        Ok(object) => serde_json::to_value(&object)
            .map_err(|error| helper_error("InvalidInput", error.to_string())),
        Err(error) => Err(helper_error("VideoEditError", error.to_string())),
    }
}
