use betterlite_models::decisions::Json;
use secrecy::SecretString;

use crate::chat::response::ChatResponse;
use crate::openai_base_config::Object;

use super::error::HostModelIdentityError;
use super::reply::{HostReplyMode, PostCall, PostCallLogger, ReplySource, decode_host_named_reply};

#[derive(Debug, Clone, Copy)]
pub struct Reply<'request> {
    pub body: &'request str,
    pub request_model: &'request str,
    pub messages: &'request Json,
    pub api_key: Option<&'request SecretString>,
    pub request_data: &'request Object,
    pub json_mode: bool,
    pub source: ReplySource,
}

// CompactifAIConfig.transform_response logs the raw text, then lets a json failure propagate unchanged
pub fn process_response(
    reply: Reply<'_>,
    logger: &mut dyn PostCallLogger,
) -> Result<ChatResponse, HostModelIdentityError> {
    logger.post_call(PostCall {
        input: reply.messages,
        api_key: reply.api_key,
        original_response: reply.body,
        complete_input_dict: reply.request_data,
    });
    let mode = HostReplyMode::Compactifai {
        request_model: reply.request_model,
        json_mode: reply.json_mode,
    };
    decode_host_named_reply(reply.body.as_bytes(), mode, &reply.source)
}
