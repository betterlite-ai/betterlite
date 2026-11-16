use betterlite_openai::chat_content::UrlDownloader;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};

use crate::databricks_chat::error::DatabricksChatError;

const DEFAULT_FILENAME: &str = "my_file.pdf";
const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const FILE_ID: &str = "file_id";
const FILENAME: &str = "filename";
const FORMAT: &str = "format";
const IMAGE_URL: &str = "image_url";
const PDF_URL_PREFIXES: [&str; 3] = ["https://", "http://", "www."];

// The parent OpenAI _transform_messages rewrites the content items of user messages
pub(super) fn transform_user_contents(
    messages: &[Message],
    downloads: &dyn UrlDownloader,
) -> Result<Vec<Message>, DatabricksChatError> {
    let mut transformed = Vec::with_capacity(messages.len());
    for message in messages {
        let mut copy = message.clone();
        if message.role == "user"
            && let Some(Content::Parts(parts)) = &mut copy.content
        {
            for part in parts {
                transform_content_item(part, downloads)?;
            }
        }
        transformed.push(copy);
    }
    Ok(transformed)
}

fn transform_content_item(
    part: &mut ContentPart,
    downloads: &dyn UrlDownloader,
) -> Result<(), DatabricksChatError> {
    match part.kind.as_str() {
        "image_url" => transform_image_url(&mut part.fields),
        "file" => transform_file(&mut part.fields, downloads),
        _ => Ok(()),
    }
}

// A string image_url becomes {"url": ...}; an object loses the litellm-only format key
fn transform_image_url(fields: &mut Object) -> Result<(), DatabricksChatError> {
    match fields.get_mut(IMAGE_URL) {
        None => Err(DatabricksChatError::Shape {
            message: String::from("an image_url content block is missing its image_url field"),
        }),
        Some(Json::Object(image_url)) => {
            image_url.shift_remove(FORMAT);
            Ok(())
        }
        Some(image_url @ Json::Text(_)) => {
            let mut wrapped = Object::new();
            wrapped.insert(String::from("url"), image_url.clone());
            *image_url = Json::Object(wrapped);
            Ok(())
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => Ok(()),
    }
}

// _transform_content_item: a PDF URL file_id is downloaded, a file_data without a filename gets one
fn transform_file(
    fields: &mut Object,
    downloads: &dyn UrlDownloader,
) -> Result<(), DatabricksChatError> {
    let Some(Json::Object(file)) = fields.get_mut(FILE) else {
        return Err(DatabricksChatError::MissingFile);
    };
    file.shift_remove(FORMAT);
    if file.is_empty() {
        return Ok(());
    }
    let pdf_url = match file.get(FILE_ID) {
        Some(Json::Text(file_id))
            if PDF_URL_PREFIXES
                .iter()
                .any(|prefix| file_id.starts_with(prefix)) =>
        {
            Some(file_id.clone())
        }
        None
        | Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        ) => None,
    };
    if let Some(url) = pdf_url {
        file.insert(
            String::from(FILE_DATA),
            Json::Text(downloads.download(&url)?),
        );
        file.insert(
            String::from(FILENAME),
            Json::Text(String::from(DEFAULT_FILENAME)),
        );
        file.shift_remove(FILE_ID);
    }
    if !matches!(file.get(FILE_DATA), None | Some(Json::Null))
        && matches!(file.get(FILENAME), None | Some(Json::Null))
    {
        file.insert(
            String::from(FILENAME),
            Json::Text(String::from(DEFAULT_FILENAME)),
        );
    }
    Ok(())
}
