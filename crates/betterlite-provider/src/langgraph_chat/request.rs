use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message, convert_content_list_to_str};

use super::error::LangGraphFailure;
use super::records::{LangGraphInput, LangGraphMessage, LangGraphRequest, LangGraphRole};

const DEFAULT_STREAM_MODE: &str = "messages-tuple";

// an empty assistant_id counts as unset, like LiteLLM's truthiness test
pub fn assistant_id(model: &str, options: &Object) -> Result<String, LangGraphFailure> {
    match options.get("assistant_id") {
        Some(Json::Text(id)) if !id.is_empty() => return Ok(id.clone()),
        Some(Json::Text(_) | Json::Null) | None => {}
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(LangGraphFailure::OptionType {
                field: "assistant_id",
            });
        }
    }
    Ok(model.split_once('/').map_or(model, |(_, id)| id).to_owned())
}

#[must_use]
pub fn convert_messages(messages: &[Message]) -> Vec<LangGraphMessage> {
    messages
        .iter()
        .map(|message| {
            let role = match message.role.as_str() {
                "assistant" => LangGraphRole::Assistant,
                "system" => LangGraphRole::System,
                _ => LangGraphRole::Human,
            };
            let content = match &message.content {
                Some(Content::Text(text)) => text.clone(),
                Some(Content::Parts(_)) => convert_content_list_to_str(message),
                None => String::new(),
            };
            let metadata = match message.provider_specific_fields.get("metadata") {
                Some(Json::Object(metadata)) if !metadata.is_empty() => Some(metadata.clone()),
                _ => None,
            };
            LangGraphMessage {
                role,
                content,
                metadata,
            }
        })
        .collect()
}

fn stream_mode(options: &Object) -> Result<Json, LangGraphFailure> {
    match options.get("stream_mode") {
        None => Ok(Json::Text(String::from(DEFAULT_STREAM_MODE))),
        Some(mode @ (Json::Text(_) | Json::Null)) => Ok(mode.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(LangGraphFailure::OptionType {
                field: "stream_mode",
            })
        }
    }
}

fn object_or_null(options: &Object, field: &'static str) -> Result<Option<Json>, LangGraphFailure> {
    match options.get(field) {
        None => Ok(None),
        Some(value @ (Json::Object(_) | Json::Null)) => Ok(Some(value.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(LangGraphFailure::OptionType { field })
        }
    }
}

// stream comes from litellm_params, not from the options: a stream option alone adds no stream_mode
pub fn transform_request(
    model: &str,
    messages: &[Message],
    options: &Object,
    stream: bool,
) -> Result<LangGraphRequest, LangGraphFailure> {
    Ok(LangGraphRequest {
        assistant_id: assistant_id(model, options)?,
        input: LangGraphInput {
            messages: convert_messages(messages),
        },
        stream_mode: if stream {
            Some(stream_mode(options)?)
        } else {
            None
        },
        config: object_or_null(options, "config")?,
        metadata: object_or_null(options, "metadata")?,
        thread_id: options.get("thread_id").cloned(),
    })
}
