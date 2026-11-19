pub mod assistant;
pub mod error;
pub mod user;

mod tests;

use betterlite_anthropic::message_replay::{BlockStrip, strip_blocks};
use betterlite_models::model_capability_predicates::CapabilityPredicates;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::api::messages_chat_options::system::mid_turn_system_message;
use crate::api::messages_content_primitives::markers::mid_system_supported;
use crate::api::messages_native_prepare::system_turns::{
    convert_mid_system_turns, is_system_role_message,
};

use self::error::ChatContentError;

// The stateless adapter holds nothing else: every turn is shaped for this target alone.
pub struct ChatTarget<'a> {
    pub model: Option<&'a str>,
    pub custom_llm_provider: Option<&'a str>,
}

// A trailing system turn keeps its role only when the caller or the model's capability says so.
pub async fn translate_messages<Predicates>(
    messages: &[Json],
    target: &ChatTarget<'_>,
    preserve_midturn_system: bool,
    predicates: &mut Predicates,
) -> Result<Vec<Json>, ChatContentError>
where
    Predicates: CapabilityPredicates + Send,
{
    let mut leading = strip_blocks(messages, BlockStrip::EncryptedReasoning);
    let leading_count = leading
        .iter()
        .position(|message| !is_system_role_message(message))
        .unwrap_or(leading.len());
    let trailing = leading.split_off(leading_count);
    let keeps_midturn_system = preserve_midturn_system
        || !trailing.iter().any(is_system_role_message)
        || mid_system_supported(predicates, target.model, target.custom_llm_provider).await;
    let ordered = if keeps_midturn_system {
        leading.extend(trailing);
        leading
    } else {
        let mut converted = leading;
        converted.extend(convert_mid_system_turns(&trailing));
        converted
    };
    let mut new_messages = Vec::with_capacity(ordered.len());
    for message in &ordered {
        push_message(message, target, &mut new_messages)?;
    }
    Ok(new_messages)
}

fn push_message(
    message: &Json,
    target: &ChatTarget<'_>,
    new_messages: &mut Vec<Json>,
) -> Result<(), ChatContentError> {
    let Some(fields) = message.as_object() else {
        return Ok(());
    };
    match fields.get("role").and_then(Json::as_str) {
        Some("system") => {
            if let Some(system) = mid_turn_system_message(fields, target.model) {
                new_messages.push(Json::Object(system));
            }
        }
        Some("user") => user::push_user_messages(fields, target, new_messages)?,
        Some("assistant") => assistant::push_assistant_message(fields, target, new_messages)?,
        Some(_) | None => {}
    }
    Ok(())
}

fn text_part(block: &Object) -> Object {
    let text = block.get("text").and_then(Json::as_str).unwrap_or_default();
    let mut part = Object::new();
    part.insert("type".to_owned(), Json::Text("text".to_owned()));
    part.insert("text".to_owned(), Json::Text(text.to_owned()));
    part
}

// A part built from our own primitives always encodes, but the caller still sees the failure.
fn encode<Part>(part: &Part) -> Result<Json, ChatContentError>
where
    Part: Serialize,
{
    let encoded = serde_json::to_value(part)?;
    Ok(serde_json::from_value(encoded)?)
}
