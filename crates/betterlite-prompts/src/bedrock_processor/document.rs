use sha2::{Digest as _, Sha256};

use crate::arguments::json::{Json, Object};
use crate::bedrock_images::formats::validate_format;

use super::error::ProcessorError;

const CONTENT_HASH_CHARS: usize = 16;
const DATA: &str = "data";
const DOCUMENT: &str = "document";
const FORMAT: &str = "format";
const HASH_SAMPLE_BYTES: usize = 64 * 1024;
const MEDIA_TYPE: &str = "media_type";
const NAME: &str = "name";
const SOURCE: &str = "source";
const SOURCE_BYTES: &str = "bytes";

// Python's str.split() drops the same whitespace U+001C..U+001F that is_blank covers.
fn content_hash(data: &str) -> String {
    let normalized: String = data
        .split(|character: char| {
            character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
        })
        .collect();
    let bytes = normalized.as_bytes();
    let sample = bytes.get(..HASH_SAMPLE_BYTES).unwrap_or(bytes);
    let mut hasher = Sha256::new();
    hasher.update(sample);
    hasher.update(normalized.len().to_string().as_bytes());
    let digest = hex::encode(hasher.finalize());
    digest
        .get(..CONTENT_HASH_CHARS)
        .unwrap_or(&digest)
        .to_owned()
}

fn document_payload(fields: &Object, block: usize) -> Result<(&str, &str), ProcessorError> {
    let source = match fields.get(SOURCE) {
        None => return Err(ProcessorError::DocumentSourceMissing { block }),
        Some(Json::Object(source)) => source,
        Some(_) => return Err(ProcessorError::DocumentSourceNotObject { block }),
    };
    if source.get("type").and_then(Json::as_str) != Some("base64") {
        return Err(ProcessorError::DocumentSourceNotBase64 { block });
    }
    let media_type = match source.get(MEDIA_TYPE) {
        None => return Err(ProcessorError::DocumentMediaTypeMissing { block }),
        Some(Json::Text(value)) => value.as_str(),
        Some(_) => return Err(ProcessorError::DocumentMediaTypeNotText { block }),
    };
    let data = match source.get(DATA) {
        None => return Err(ProcessorError::DocumentDataMissing { block }),
        Some(Json::Text(value)) => value.as_str(),
        Some(_) => return Err(ProcessorError::DocumentDataNotText { block }),
    };
    Ok((media_type, data))
}

pub fn document_message_block(fields: &Object, block: usize) -> Result<Json, ProcessorError> {
    let (media_type, data) = document_payload(fields, block)?;
    let subtype = media_type
        .split('/')
        .nth(1)
        .ok_or(ProcessorError::DocumentMediaWithoutSubtype { block })?;
    let document_format =
        validate_format(media_type, subtype).map_err(|failure| ProcessorError::DocumentFormat {
            block,
            source: failure,
        })?;
    let name = format!("Document_{}_{document_format}", content_hash(data));
    Ok(Json::Object(Object::from_iter([(
        DOCUMENT.to_owned(),
        Json::Object(Object::from_iter([
            (FORMAT.to_owned(), Json::Text(document_format)),
            (
                SOURCE.to_owned(),
                Json::Object(Object::from_iter([(
                    SOURCE_BYTES.to_owned(),
                    Json::Text(data.to_owned()),
                )])),
            ),
            (NAME.to_owned(), Json::Text(name)),
        ])),
    )])))
}
