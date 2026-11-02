use crate::arguments::json::{Json, Object};
use crate::message_content::ContentPart;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FileMigrationError {
    #[error("content block has type='file' but is missing the required 'file' field")]
    MissingFile,
    #[error("file_id and file_data are both missing")]
    MissingFileReference,
}

// LiteLLM crashes here when file is present but not a dict; betterlite reports MissingFile instead.
#[must_use = "returns the migrated image content part"]
pub fn migrate_file_to_image_url(part: &ContentPart) -> Result<ContentPart, FileMigrationError> {
    let file = part
        .fields
        .get("file")
        .and_then(Json::as_object)
        .ok_or(FileMigrationError::MissingFile)?;
    let file_id = file.get("file_id").and_then(Json::as_str);
    let file_data = file.get("file_data").and_then(Json::as_str);
    let file_format = file.get("format").and_then(Json::as_str);
    let url = file_id
        .or(file_data)
        .ok_or(FileMigrationError::MissingFileReference)?;

    let mut image_url = Object::new();
    image_url.insert("url".to_owned(), Json::Text(url.to_owned()));
    if let Some(format_value) = file_format {
        image_url.insert("format".to_owned(), Json::Text(format_value.to_owned()));
    }
    let mut fields = Object::new();
    fields.insert("image_url".to_owned(), Json::Object(image_url));
    Ok(ContentPart {
        kind: "image_url".to_owned(),
        fields,
    })
}

mod tests;
