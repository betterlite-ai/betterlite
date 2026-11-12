use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message, convert_content_list_to_str};
use serde::Serialize;

use super::error::{LangFlowError, TWEAKS_MESSAGE};

const CHAT_TYPE: &str = "chat";
const INPUT_TYPE_FIELD: &str = "input_type";
const OUTPUT_TYPE_FIELD: &str = "output_type";
const SESSION_ID_FIELD: &str = "session_id";
const TWEAKS_FIELD: &str = "tweaks";
const USER_ROLE: &str = "user";

pub struct RequestInput<'request> {
    pub messages: &'request [Message],
    pub optional_params: &'request Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestBody {
    pub input_value: String,
    pub input_type: Json,
    pub output_type: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Json>,
}

pub fn reject_tweaks(params: &Object) -> Result<(), LangFlowError> {
    if params
        .get(TWEAKS_FIELD)
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return Err(LangFlowError::Api {
            status_code: 400,
            message: String::from(TWEAKS_MESSAGE),
        });
    }
    Ok(())
}

pub fn transform_request(input: &RequestInput<'_>) -> Result<RequestBody, LangFlowError> {
    reject_tweaks(input.optional_params)?;
    Ok(RequestBody {
        input_value: last_message_text(input.messages),
        input_type: field_or_default(input.optional_params, INPUT_TYPE_FIELD, CHAT_TYPE),
        output_type: field_or_default(input.optional_params, OUTPUT_TYPE_FIELD, CHAT_TYPE),
        session_id: input
            .optional_params
            .get(SESSION_ID_FIELD)
            .filter(|value| value.is_truthy())
            .cloned(),
    })
}

fn field_or_default(optional_params: &Object, name: &str, default: &str) -> Json {
    match optional_params.get(name) {
        Some(value) if !matches!(value, Json::Null) => value.clone(),
        Some(Json::Null) | None => Json::Text(default.to_owned()),
    }
}

fn last_message_text(messages: &[Message]) -> String {
    let last_user = messages
        .iter()
        .rev()
        .find(|message| message.role == USER_ROLE);
    last_user
        .or_else(|| messages.last())
        .map_or_else(String::new, message_text)
}

fn message_text(message: &Message) -> String {
    // LiteLLM flattens only a list content through the shared helper; text keeps any search_results out
    match message.content.as_ref() {
        Some(Content::Text(text)) => text.clone(),
        Some(Content::Parts(_)) => convert_content_list_to_str(message),
        None => String::new(),
    }
}
