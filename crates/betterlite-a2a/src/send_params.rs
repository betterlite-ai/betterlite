pub mod error;
mod shape;
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
    let prepared = shape::prepare(params)?;
    if let Ok(parsed) = MessageSendParamsV03::deserialize(&prepared) {
        return Ok(parsed);
    }
    v10::parse_and_convert(&prepared)
}
