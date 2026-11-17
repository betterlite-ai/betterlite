pub mod access;
pub mod error;
pub mod guarded;
pub mod orphans;
pub mod system;
mod tests;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;

use crate::converse_messages::error::ConverseMessageError;
use crate::model_routes::ParamMap;

#[derive(Clone, Copy, Debug)]
pub struct ConverseMessagePolicy<'request> {
    pub model: Option<&'request str>,
    pub model_cost: &'request ParamMap,
    pub neutralize: bool,
    pub modify_params: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransformedMessages {
    pub messages: Vec<Message>,
    pub system: Vec<Json>,
}

// LiteLLM's async request assembly applies these three policies in exactly this order.
pub fn transform_messages(
    messages: &[Message],
    optional_params: &mut ParamMap,
    policy: &ConverseMessagePolicy<'_>,
) -> Result<TransformedMessages, ConverseMessageError> {
    let (remaining, system) =
        system::transform_system_message(messages, policy.model, policy.model_cost)?;
    let handled = orphans::handle_orphaned_tool_blocks(
        &remaining,
        optional_params,
        orphans::OrphanPolicy {
            neutralize: policy.neutralize,
            modify_params: policy.modify_params,
        },
    )?;
    let messages = guarded::guard_trailing_user_turns(&handled, optional_params)?;
    Ok(TransformedMessages { messages, system })
}
