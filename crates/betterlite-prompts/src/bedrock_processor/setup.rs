use crate::arguments::json::{Json, Object};

use super::blocks::{ASSISTANT, CONTENT, ROLE, Role, USER, role_at};
use super::error::ProcessorError;

const DEFAULT_CONTINUE_TEXT: &str = "Please continue.";
const PREFIX: &str = "prefix";

fn default_user_continue_message() -> Json {
    Json::Object(Object::from_iter([
        (ROLE.to_owned(), Json::Text(USER.to_owned())),
        (
            CONTENT.to_owned(),
            Json::Text(DEFAULT_CONTINUE_TEXT.to_owned()),
        ),
    ]))
}

fn needs_continue(message: &Json, index: usize) -> Result<bool, ProcessorError> {
    let prefix = message.as_object().and_then(|fields| fields.get(PREFIX));
    match (role_at(message, index)?, prefix) {
        (Role::Text(ASSISTANT), None | Some(Json::Bool(false) | Json::Null)) => Ok(true),
        (Role::Text(ASSISTANT), Some(Json::Bool(true))) => Ok(false),
        (Role::Text(ASSISTANT), Some(_)) => Err(ProcessorError::PrefixNotBool { message: index }),
        (Role::Text(_) | Role::Null, _) => Ok(false),
    }
}

pub fn initial_message_setup(
    messages: &[Json],
    user_continue_message: Option<&Json>,
    modify_params: bool,
) -> Result<Vec<Json>, ProcessorError> {
    let continue_message = match user_continue_message {
        None | Some(Json::Null) => None,
        Some(value) => Some(value),
    };
    let mut prepared = messages.to_vec();
    if prepared.is_empty() {
        match continue_message {
            Some(value) => prepared.push(value.clone()),
            None if modify_params => prepared.push(default_user_continue_message()),
            None => return Err(ProcessorError::NoMessages),
        }
    }
    if let Some(first) = prepared.first()
        && needs_continue(first, 0)?
    {
        match continue_message {
            Some(value) => prepared.insert(0, value.clone()),
            None if modify_params => prepared.insert(0, default_user_continue_message()),
            None => {}
        }
    }
    let last_index = prepared.len().saturating_sub(1);
    if let Some(last) = prepared.get(last_index)
        && needs_continue(last, last_index)?
    {
        match continue_message {
            Some(value) => prepared.push(value.clone()),
            None if modify_params => prepared.push(default_user_continue_message()),
            None => {}
        }
    }
    Ok(prepared)
}
