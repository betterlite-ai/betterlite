use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};

use crate::databricks_chat::error::DatabricksChatError;

const DEFAULT_FILENAME: &str = "my_file.pdf";
const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const FILENAME: &str = "filename";
const FORMAT: &str = "format";
const IMAGE_URL: &str = "image_url";
const USER: &str = "user";

// The parent OpenAI message transform rewrites the content items of user messages
pub(super) fn transform_user_contents(
    messages: &[Message],
) -> Result<Vec<Message>, DatabricksChatError> {
    let mut transformed = Vec::with_capacity(messages.len());
    for message in messages {
        transformed.push(transform_user_content(message)?);
    }
    Ok(transformed)
}

fn transform_user_content(message: &Message) -> Result<Message, DatabricksChatError> {
    if message.role != USER {
        return Ok(message.clone());
    }
    let Some(Content::Parts(parts)) = &message.content else {
        return Ok(message.clone());
    };
    if parts.is_empty() {
        return Ok(message.clone());
    }
    let mut converted = Vec::with_capacity(parts.len());
    for part in parts {
        converted.push(transform_content_item(part)?);
    }
    let mut transformed = message.clone();
    transformed.content = Some(Content::Parts(converted));
    Ok(transformed)
}

fn transform_content_item(part: &ContentPart) -> Result<ContentPart, DatabricksChatError> {
    let mut transformed = part.clone();
    match part.kind.as_str() {
        "image_url" => transform_image_url(&mut transformed)?,
        FILE => transform_file(&mut transformed)?,
        _ => {}
    }
    Ok(transformed)
}

// A string image_url becomes {"url": ...}; an object loses the litellm-only format key
fn transform_image_url(item: &mut ContentPart) -> Result<(), DatabricksChatError> {
    let Some(image_url) = item.fields.get(IMAGE_URL) else {
        return Err(DatabricksChatError::Shape {
            message: String::from("an image_url content block is missing its image_url field"),
        });
    };
    let replacement = match image_url {
        Json::Text(url) => {
            let mut fields = Object::new();
            fields.insert("url".to_owned(), Json::Text(url.clone()));
            Json::Object(fields)
        }
        Json::Object(fields) => {
            let mut stripped = fields.clone();
            stripped.shift_remove(FORMAT);
            Json::Object(stripped)
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => image_url.clone(),
    };
    item.fields.insert(IMAGE_URL.to_owned(), replacement);
    Ok(())
}

// transform_file: the format key goes, a file_data without a filename gets the default one
fn transform_file(item: &mut ContentPart) -> Result<(), DatabricksChatError> {
    let Some(file) = item.fields.get(FILE) else {
        return Err(missing_file());
    };
    let Json::Object(fields) = file else {
        if matches!(file, Json::Null) {
            return Err(missing_file());
        }
        return Err(DatabricksChatError::Shape {
            message: String::from("the file of a file content block must be an object"),
        });
    };
    let mut file_object = fields.clone();
    file_object.shift_remove(FORMAT);
    if !file_object.is_empty()
        && !matches!(file_object.get(FILE_DATA), None | Some(Json::Null))
        && matches!(file_object.get(FILENAME), None | Some(Json::Null))
    {
        file_object.insert(FILENAME.to_owned(), Json::Text(DEFAULT_FILENAME.to_owned()));
    }
    item.fields
        .insert(FILE.to_owned(), Json::Object(file_object));
    Ok(())
}

fn missing_file() -> DatabricksChatError {
    DatabricksChatError::BadRequest {
        message: String::from(
            "content block has type='file' but is missing the required 'file' field",
        ),
    }
}
