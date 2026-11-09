use crate::arguments::json::{Json, Object};

use super::empty_blocks::{process_empty_text_blocks, skip_empty_text_blocks};
use super::error::ContentError;
use super::messages::{
    CONTENT, DEFAULT_CONTINUE_TEXT, ROLE, TEXT, TEXT_TYPE, TYPE, content_list_to_str,
    has_non_content_values, is_blank,
};

const ASSISTANT: &str = "assistant";
const USER: &str = "user";

#[must_use]
pub fn assistant_continue_message(continue_message: Option<&Json>) -> Object {
    match continue_message {
        Some(Json::Text(text)) if !text.is_empty() => Object::from_iter([
            (ROLE.to_owned(), Json::Text(ASSISTANT.to_owned())),
            (CONTENT.to_owned(), Json::Text(text.clone())),
        ]),
        Some(Json::Object(fields)) if !fields.is_empty() => fields.clone(),
        None
        | Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Text(_),
        ) => Object::from_iter([
            (ROLE.to_owned(), Json::Text(ASSISTANT.to_owned())),
            (
                CONTENT.to_owned(),
                Json::List(vec![Json::Object(Object::from_iter([
                    (TYPE.to_owned(), Json::Text(TEXT_TYPE.to_owned())),
                    (
                        TEXT.to_owned(),
                        Json::Text(DEFAULT_CONTINUE_TEXT.to_owned()),
                    ),
                ]))]),
            ),
        ]),
    }
}

#[must_use]
pub fn insert_assistant_continue(
    messages: &[Json],
    continue_message: Option<&Json>,
    modify_params: bool,
) -> Vec<Json> {
    let mut extended = messages.to_vec();
    let continue_text = match continue_message {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Object(fields)) => Some(content_list_to_str(fields)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => None,
        None | Some(Json::Null) => modify_params.then(|| DEFAULT_CONTINUE_TEXT.to_owned()),
    };
    let Some(text) = continue_text else {
        return extended;
    };
    extended.push(Json::Object(Object::from_iter([
        (ROLE.to_owned(), Json::Text(ASSISTANT.to_owned())),
        (
            CONTENT.to_owned(),
            Json::List(vec![Json::Object(Object::from_iter([(
                TEXT.to_owned(),
                Json::Text(text),
            )]))]),
        ),
    ])));
    extended
}

pub fn user_message_block_or_continue(
    message: &Json,
    continue_message: Option<&Json>,
    modify_params: bool,
) -> Result<Json, ContentError> {
    let Some(fields) = message.as_object() else {
        return Err(ContentError::MessageNotObject);
    };
    let content = fields.get(CONTENT);
    if matches!(continue_message, None | Some(Json::Null)) && !modify_params {
        return skip_empty_text_blocks(message);
    }
    let missing_content = matches!(content, None | Some(Json::Null));
    let blank_text = content.and_then(Json::as_str).is_some_and(is_blank);
    if missing_content || blank_text {
        return Ok(Json::Object(user_continue_message(continue_message)?));
    }
    if matches!(content, Some(Json::Text(_))) {
        return Ok(message.clone());
    }
    let Some(Json::List(parts)) = content else {
        return Err(ContentError::UnsupportedContent);
    };
    if parts.is_empty() {
        return Ok(Json::Object(user_continue_message(continue_message)?));
    }
    replace_empty_user_text(fields, parts, continue_message)
}

pub fn assistant_message_or_continue(
    message: &Json,
    continue_message: Option<&Json>,
    modify_params: bool,
) -> Result<Json, ContentError> {
    let Some(fields) = message.as_object() else {
        return Err(ContentError::MessageNotObject);
    };
    if matches!(continue_message, None | Some(Json::Null)) && !modify_params {
        return skip_empty_text_blocks(message);
    }
    match fields.get(CONTENT) {
        None | Some(Json::Null) => Ok(message.clone()),
        Some(Json::Text(text)) => Ok(assistant_text_content(
            message,
            fields,
            text,
            continue_message,
        )),
        Some(Json::List(_)) => process_empty_text_blocks(message, continue_message),
        Some(Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            Err(ContentError::UnsupportedContent)
        }
    }
}

fn assistant_text_content(
    message: &Json,
    fields: &Object,
    text: &str,
    continue_message: Option<&Json>,
) -> Json {
    if !is_blank(text) {
        return message.clone();
    }
    if has_non_content_values(fields) {
        let mut modified = fields.clone();
        modified.insert(CONTENT.to_owned(), Json::Null);
        return Json::Object(modified);
    }
    Json::Object(assistant_continue_message(continue_message))
}

fn user_continue_message(continue_message: Option<&Json>) -> Result<Object, ContentError> {
    if let Some(Json::Object(fields)) = continue_message
        && !fields.is_empty()
    {
        return Ok(fields.clone());
    }
    if matches!(
        continue_message,
        Some(Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_))
    ) {
        return Err(ContentError::ContinueMessageNotObject);
    }
    let mut message = Object::new();
    message.insert(ROLE.to_owned(), Json::Text(USER.to_owned()));
    message.insert(
        CONTENT.to_owned(),
        Json::Text(DEFAULT_CONTINUE_TEXT.to_owned()),
    );
    Ok(message)
}

fn replace_empty_user_text(
    fields: &Object,
    parts: &[Json],
    continue_message: Option<&Json>,
) -> Result<Json, ContentError> {
    let mut modified_parts: Vec<Json> = Vec::with_capacity(parts.len());
    let mut replaced = false;
    for (index, part) in parts.iter().enumerate() {
        if replaced {
            modified_parts.push(part.clone());
            continue;
        }
        let Some(part_fields) = part.as_object() else {
            return Err(ContentError::PartNotObject { index });
        };
        let Some(kind) = part_fields.get(TYPE) else {
            return Err(ContentError::PartWithoutType { index });
        };
        if kind.as_str() != Some(TEXT_TYPE) {
            modified_parts.push(part.clone());
            continue;
        }
        let text_value = part_fields
            .get(TEXT)
            .ok_or(ContentError::TextPartWithoutText { index })?;
        let Json::Text(text) = text_value else {
            return Err(ContentError::TextPartNotText { index });
        };
        if !is_blank(text) {
            modified_parts.push(part.clone());
            continue;
        }
        let continue_fields = user_continue_message(continue_message)?;
        let mut replaced_part = part_fields.clone();
        replaced_part.insert(
            TEXT.to_owned(),
            Json::Text(content_list_to_str(&continue_fields)),
        );
        modified_parts.push(Json::Object(replaced_part));
        replaced = true;
    }
    let mut modified = fields.clone();
    modified.insert(CONTENT.to_owned(), Json::List(modified_parts));
    Ok(Json::Object(modified))
}
