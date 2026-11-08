use serde::Serialize;
use thiserror::Error;

use crate::anthropic_results::images::{ImageChunk, ImageError, to_anthropic_image_chunk};
use crate::arguments::json::{Json, Object};
use crate::media::inline::RemoteMediaFetcher;

const TYPE: &str = "type";
const SOURCE: &str = "source";
const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const FILE_ID: &str = "file_id";
const FORMAT: &str = "format";
const MEDIA_TYPE: &str = "media_type";
const DATA: &str = "data";
const URL: &str = "url";
const DOCUMENT: &str = "document";
const IMAGE: &str = "image";
const CONTAINER_UPLOAD: &str = "container_upload";
const BASE64: &str = "base64";
const PDF: &str = "application/pdf";
const TEXT_PLAIN: &str = "text/plain";
const HTTP_PREFIX: &str = "http";
const IMAGE_FORMATS: [&str; 4] = ["image/jpeg", "image/png", "image/gif", "image/webp"];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentBlockType {
    Document,
    Image,
    ContainerUpload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIdContentType {
    DocumentUrl,
    ContainerUpload,
}

#[derive(Debug, Error)]
pub enum FileMessageError {
    #[error("Content block has type='file' but is missing the required 'file' field")]
    MissingFile,
    #[error("file message field {field} must be a string")]
    Field { field: &'static str },
    #[error("either file_data or file_id must be present in the file message")]
    MissingSource,
    #[error(transparent)]
    Image(#[from] ImageError),
}

#[must_use]
pub fn select_content_block_type(format: &str) -> ContentBlockType {
    if format == PDF || format == TEXT_PLAIN {
        ContentBlockType::Document
    } else if IMAGE_FORMATS.contains(&format) {
        ContentBlockType::Image
    } else {
        ContentBlockType::ContainerUpload
    }
}

#[must_use]
pub fn infer_file_id_content_type(file_id: &str) -> FileIdContentType {
    if file_id.starts_with(HTTP_PREFIX) {
        FileIdContentType::DocumentUrl
    } else {
        FileIdContentType::ContainerUpload
    }
}

#[must_use]
pub fn content_element_factory(chunk: &ImageChunk) -> Object {
    base64_block(
        if chunk.media_type == PDF {
            DOCUMENT
        } else {
            IMAGE
        },
        &chunk.media_type,
        &chunk.data,
    )
}

pub async fn process_openai_file_message<Source>(
    message: &Object,
    fetcher: &Source,
) -> Result<Object, FileMessageError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let file = match message.get(FILE) {
        Some(Json::Object(file)) => file,
        None | Some(Json::Null) => return Err(FileMessageError::MissingFile),
        Some(_) => return Err(FileMessageError::Field { field: FILE }),
    };
    let format = match file.get(FORMAT) {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(_) => return Err(FileMessageError::Field { field: FORMAT }),
    };
    let file_data = match file.get(FILE_DATA) {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(_) => return Err(FileMessageError::Field { field: FILE_DATA }),
    };
    if let Some(data) = file_data {
        let chunk = to_anthropic_image_chunk(data, format, fetcher).await?;
        return Ok(base64_block(DOCUMENT, &chunk.media_type, &chunk.data));
    }
    let file_id = match file.get(FILE_ID) {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(_) => return Err(FileMessageError::Field { field: FILE_ID }),
    };
    let Some(reference) = file_id else {
        return Err(FileMessageError::MissingSource);
    };
    Ok(referenced_block(reference, format))
}

fn referenced_block(file_id: &str, format: Option<&str>) -> Object {
    format.map_or_else(
        || match infer_file_id_content_type(file_id) {
            FileIdContentType::DocumentUrl => source_block(DOCUMENT, url_source(file_id)),
            FileIdContentType::ContainerUpload => container_block(file_id),
        },
        |media_format| match select_content_block_type(media_format) {
            ContentBlockType::Document => source_block(DOCUMENT, file_source(file_id)),
            ContentBlockType::Image => source_block(IMAGE, file_source(file_id)),
            ContentBlockType::ContainerUpload => container_block(file_id),
        },
    )
}

fn base64_block(kind: &str, media_type: &str, data: &str) -> Object {
    source_block(
        kind,
        Object::from_iter([
            (TYPE.to_owned(), Json::Text(BASE64.to_owned())),
            (MEDIA_TYPE.to_owned(), Json::Text(media_type.to_owned())),
            (DATA.to_owned(), Json::Text(data.to_owned())),
        ]),
    )
}

fn source_block(kind: &str, source: Object) -> Object {
    Object::from_iter([
        (TYPE.to_owned(), Json::Text(kind.to_owned())),
        (SOURCE.to_owned(), Json::Object(source)),
    ])
}

fn file_source(file_id: &str) -> Object {
    Object::from_iter([
        (TYPE.to_owned(), Json::Text(FILE.to_owned())),
        (FILE_ID.to_owned(), Json::Text(file_id.to_owned())),
    ])
}

fn url_source(url: &str) -> Object {
    Object::from_iter([
        (TYPE.to_owned(), Json::Text(URL.to_owned())),
        (URL.to_owned(), Json::Text(url.to_owned())),
    ])
}

fn container_block(file_id: &str) -> Object {
    Object::from_iter([
        (TYPE.to_owned(), Json::Text(CONTAINER_UPLOAD.to_owned())),
        (FILE_ID.to_owned(), Json::Text(file_id.to_owned())),
    ])
}
