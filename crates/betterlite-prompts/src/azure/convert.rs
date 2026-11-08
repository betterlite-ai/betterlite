use crate::arguments::json::{Json, Object};
use crate::azure::error::AzureError;
use crate::media::limits::{MAX_MESSAGE_PARTS, MAX_MESSAGES};
use crate::message_content::{Content, ContentPart, Message};

const ASSISTANT: &str = "assistant";
const USER: &str = "user";
const IMAGE_URL: &str = "image_url";
const URL: &str = "url";
const FILE: &str = "file";
const FORMAT: &str = "format";
const FUNCTION_CALL: &str = "function_call";
const ARGUMENTS: &str = "arguments";

pub fn convert_to_azure_openai_messages(messages: &[Message]) -> Result<Vec<Message>, AzureError> {
    if messages.len() > MAX_MESSAGES {
        return Err(AzureError::TooManyMessages {
            limit: MAX_MESSAGES,
        });
    }
    let mut converted = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        let mut next = message.clone();
        if next.role == ASSISTANT {
            convert_function_call(&mut next, index)?;
        }
        if next.role == USER {
            convert_user_content(&mut next, index)?;
        }
        converted.push(next);
    }
    Ok(converted)
}

fn convert_function_call(message: &mut Message, index: usize) -> Result<(), AzureError> {
    match message.provider_specific_fields.get_mut(FUNCTION_CALL) {
        None | Some(Json::Null) => Ok(()),
        Some(Json::Object(function_call)) => default_arguments(function_call, index),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(AzureError::FunctionCall { index })
        }
    }
}

fn default_arguments(function_call: &mut Object, index: usize) -> Result<(), AzureError> {
    // Azure reads a null arguments value as the empty string an omitted one already means
    match function_call.get(ARGUMENTS) {
        None | Some(Json::Null) => {
            function_call.insert(ARGUMENTS.to_owned(), Json::Text(String::new()));
            Ok(())
        }
        Some(Json::Text(_)) => Ok(()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Object(_) | Json::List(_)) => {
            Err(AzureError::Arguments { index })
        }
    }
}

fn convert_user_content(message: &mut Message, index: usize) -> Result<(), AzureError> {
    let Some(Content::Parts(parts)) = message.content.as_mut() else {
        return Ok(());
    };
    if parts.len() > MAX_MESSAGE_PARTS {
        return Err(AzureError::TooManyParts {
            limit: MAX_MESSAGE_PARTS,
        });
    }
    for (part_index, part) in parts.iter_mut().enumerate() {
        match part.kind.as_str() {
            IMAGE_URL => convert_image_url(part, index, part_index)?,
            FILE => convert_file(part, index, part_index)?,
            _ => {}
        }
    }
    Ok(())
}

fn convert_image_url(
    part: &mut ContentPart,
    index: usize,
    part_index: usize,
) -> Result<(), AzureError> {
    match part.fields.get(IMAGE_URL).cloned() {
        Some(Json::Text(url)) => {
            let mut image_url = Object::new();
            image_url.insert(URL.to_owned(), Json::Text(url));
            part.fields
                .insert(IMAGE_URL.to_owned(), Json::Object(image_url));
        }
        Some(Json::Object(fields)) => {
            part.fields
                .insert(IMAGE_URL.to_owned(), Json::Object(without_format(&fields)));
        }
        None => {
            return Err(AzureError::MissingImageUrl {
                index,
                part: part_index,
            });
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            return Err(AzureError::ImageUrl {
                index,
                part: part_index,
            });
        }
    }
    Ok(())
}

fn convert_file(part: &mut ContentPart, index: usize, part_index: usize) -> Result<(), AzureError> {
    match part.fields.get(FILE).cloned() {
        None => {
            part.fields
                .insert(FILE.to_owned(), Json::Object(Object::new()));
        }
        Some(Json::Object(fields)) => {
            part.fields
                .insert(FILE.to_owned(), Json::Object(without_format(&fields)));
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(AzureError::File {
                index,
                part: part_index,
            });
        }
    }
    Ok(())
}

fn without_format(fields: &Object) -> Object {
    fields
        .iter()
        .filter(|(key, _value)| key.as_str() != FORMAT)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
