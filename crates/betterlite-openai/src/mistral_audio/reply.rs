use betterlite_http::proxy::UpstreamResponse;
use serde_json::Value;

use super::error::MistralAudioError;
use crate::openai_base_config::Object;

// A Mistral reply is JSON the provider chose the size of, so the decoder reads it under a fixed limit
pub(crate) const REPLY_LIMIT: usize = 16 * 1024 * 1024;

pub(crate) fn reply_fields(response: &UpstreamResponse) -> Result<Object, MistralAudioError> {
    if response.body.len() > REPLY_LIMIT {
        return Err(MistralAudioError::ReplyTooLarge { limit: REPLY_LIMIT });
    }
    match serde_json::from_slice::<Value>(&response.body) {
        Ok(Value::Object(fields)) => Ok(fields),
        Ok(_) => Err(MistralAudioError::ReplyNotObject),
        // the upstream status is the only status LiteLLM keeps for a body it cannot read as JSON
        Err(_) => Err(MistralAudioError::ReplyNotJson {
            status: response.status,
        }),
    }
}
