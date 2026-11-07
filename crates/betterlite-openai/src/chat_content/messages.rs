use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};

use super::{ChatContentError, MissingFileError, OPENAI, UrlDownloader};
use crate::chat::ChatMessage;

const DEFAULT_FILENAME: &str = "my_file.pdf";
const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const FILE_ID: &str = "file_id";
const FILENAME: &str = "filename";
const FORMAT: &str = "format";
const IMAGE_URL: &str = "image_url";
const PDF_URL_PREFIXES: [&str; 3] = ["http://", "https://", "www."];

pub(super) fn as_json(messages: &[ChatMessage]) -> Result<Vec<Json>, ChatContentError> {
    let mut values = Vec::with_capacity(messages.len());
    for message in messages {
        let value = serde_json::to_value(message)
            .map_err(|source| ChatContentError::MessageShape { source })?;
        values.push(
            serde_json::from_value(value)
                .map_err(|source| ChatContentError::MessageShape { source })?,
        );
    }
    Ok(values)
}

pub(super) fn prompt_message(message: &ChatMessage) -> Result<Message, ChatContentError> {
    let value = serde_json::to_value(message)
        .map_err(|source| ChatContentError::MessageShape { source })?;
    serde_json::from_value(value).map_err(|source| ChatContentError::MessageShape { source })
}

pub(super) fn chat_message(message: &Message) -> Result<ChatMessage, ChatContentError> {
    let value = serde_json::to_value(message)
        .map_err(|source| ChatContentError::MessageShape { source })?;
    serde_json::from_value(value).map_err(|source| ChatContentError::MessageShape { source })
}

pub(super) fn transform_user_content(
    mut message: Message,
    downloads: &dyn UrlDownloader,
) -> Result<Message, ChatContentError> {
    if message.role != "user" {
        return Ok(message);
    }
    let Some(content) = message.content.take() else {
        return Ok(message);
    };
    let Content::Parts(parts) = content else {
        message.content = Some(content);
        return Ok(message);
    };
    if parts.is_empty() {
        message.content = Some(Content::Parts(parts));
        return Ok(message);
    }
    let mut transformed = Vec::with_capacity(parts.len());
    for part in &parts {
        transformed.push(transform_content_item(part, downloads)?);
    }
    message.content = Some(Content::Parts(transformed));
    Ok(message)
}

pub(super) fn transform_content_item(
    item: &ContentPart,
    downloads: &dyn UrlDownloader,
) -> Result<ContentPart, ChatContentError> {
    let mut fields = item.fields.clone();
    match item.kind.as_str() {
        "image_url" => transform_image_url(&mut fields),
        "file" => transform_file(&mut fields, downloads)?,
        _ => {}
    }
    Ok(ContentPart {
        kind: item.kind.clone(),
        fields,
    })
}

fn transform_image_url(fields: &mut Object) {
    match fields.get(IMAGE_URL).cloned() {
        Some(Json::Text(url)) => {
            fields.insert(
                IMAGE_URL.to_owned(),
                Json::Object(Object::from_iter([("url".to_owned(), Json::Text(url))])),
            );
        }
        Some(Json::Object(mut image_url)) => {
            image_url.shift_remove(FORMAT);
            fields.insert(IMAGE_URL.to_owned(), Json::Object(image_url));
        }
        Some(other) => {
            fields.insert(IMAGE_URL.to_owned(), other);
        }
        None => {}
    }
}

fn transform_file(
    fields: &mut Object,
    downloads: &dyn UrlDownloader,
) -> Result<(), ChatContentError> {
    let Some(Json::Object(file_fields)) = fields.get(FILE) else {
        return Err(ChatContentError::MissingFile(MissingFileError {
            provider: OPENAI,
            model: None,
        }));
    };
    let mut file = file_fields.clone();
    file.shift_remove(FORMAT);
    // Python skips the default filename and the PDF download for a file object that is empty
    if !file.is_empty() {
        if contains_pdf_url(&file) {
            handle_pdf_url(&mut file, downloads)?;
        }
        if !matches!(file.get(FILE_DATA), None | Some(Json::Null))
            && matches!(file.get(FILENAME), None | Some(Json::Null))
        {
            file.insert(FILENAME.to_owned(), Json::Text(DEFAULT_FILENAME.to_owned()));
        }
    }
    fields.insert(FILE.to_owned(), Json::Object(file));
    Ok(())
}

fn contains_pdf_url(file: &Object) -> bool {
    let Some(Json::Text(file_id)) = file.get(FILE_ID) else {
        return false;
    };
    PDF_URL_PREFIXES
        .iter()
        .any(|prefix| file_id.starts_with(prefix))
}

fn handle_pdf_url(
    file: &mut Object,
    downloads: &dyn UrlDownloader,
) -> Result<(), ChatContentError> {
    let Some(Json::Text(file_id)) = file.get(FILE_ID) else {
        return Ok(());
    };
    let data = downloads.download(file_id)?;
    file.insert(FILE_DATA.to_owned(), Json::Text(data));
    file.insert(FILENAME.to_owned(), Json::Text(DEFAULT_FILENAME.to_owned()));
    file.shift_remove(FILE_ID);
    Ok(())
}
