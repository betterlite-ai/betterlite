pub mod error;
mod tests;
pub mod v03;
mod v10;

use betterlite_core::agents::json::AgentJson;
use serde::Deserialize as _;

use crate::send_params::error::SendParamsError;
use crate::send_params::v03::MessageSendParamsV03;

pub fn build_message_send_params(
    params: &AgentJson,
) -> Result<MessageSendParamsV03, SendParamsError> {
    if let Ok(parsed) = MessageSendParamsV03::deserialize(params) {
        return Ok(parsed);
    }
    if !params.is_object() {
        return Err(SendParamsError::NotAnObject);
    }
    v10::parse_and_convert(params)
}
