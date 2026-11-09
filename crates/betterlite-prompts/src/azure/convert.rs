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

pub fn to_azure(messages: &[Message]) -> Result<Vec<Message>, AzureError> {
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
    let Some(value) = message.provider_specific_fields.get_mut(FUNCTION_CALL) else {
        return Ok(());
    };
    if matches!(value, Json::Null) {
        return Ok(());
    }
    let Json::Object(function_call) = value else {
        return Err(AzureError::FunctionCall { index });
    };
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
        convert_part(part, index, part_index)?;
    }
    Ok(())
}

fn convert_part(part: &mut ContentPart, index: usize, part_index: usize) -> Result<(), AzureError> {
    match part.kind.as_str() {
        IMAGE_URL => {
            let Some(image_url) = part.fields.get(IMAGE_URL).cloned() else {
                return Err(AzureError::MissingImageUrl {
                    index,
                    part: part_index,
                });
            };
            let converted = match image_url {
                Json::Text(url) => Json::Object(Object::from([(URL.to_owned(), Json::Text(url))])),
                Json::Object(fields) => Json::Object(without_format(&fields)),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                    return Err(AzureError::ImageUrl {
                        index,
                        part: part_index,
                    });
                }
            };
            part.fields.insert(IMAGE_URL.to_owned(), converted);
        }
        FILE => {
            let converted = match part.fields.get(FILE).cloned() {
                None => Json::Object(Object::new()),
                Some(Json::Object(fields)) => Json::Object(without_format(&fields)),
                Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
                ) => {
                    return Err(AzureError::File {
                        index,
                        part: part_index,
                    });
                }
            };
            part.fields.insert(FILE.to_owned(), converted);
        }
        _ => {}
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
