use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::chat_choices::decode_chat_choices;
use betterlite_openai::chat_choices::wire::TransformedChoice;
use betterlite_openai::openai_base_config::Object;
use uuid::Uuid;

use super::error::CopilotChatError;
use super::native_reply::{NativeReply, transform_parsed_response};

#[derive(Debug)]
pub struct ChatReply {
    pub response: UpstreamResponse,
    pub choices: Vec<TransformedChoice>,
}

// A reply that is not JSON, or whose choices are filled, goes to the parent decoder as it came, so a
// failure there carries the original response. A repaired reply keeps its status and headers.
pub fn transform_response(
    mut response: UpstreamResponse,
    optional_params: Option<&Object>,
    json_mode: bool,
    identifier: fn() -> Uuid,
) -> Result<ChatReply, CopilotChatError> {
    if let Ok(reply) = serde_json::from_slice::<NativeReply>(&response.body)
        && !reply.has_choices()
    {
        let repaired = transform_parsed_response(reply)?;
        response.body = serde_json::to_vec(&repaired).map_err(CopilotChatError::Encode)?;
    }
    match decode_chat_choices(&response.body, optional_params, json_mode, identifier) {
        Ok(choices) => Ok(ChatReply { response, choices }),
        Err(source) => Err(CopilotChatError::Parent {
            source,
            response: Box::new(response),
        }),
    }
}
