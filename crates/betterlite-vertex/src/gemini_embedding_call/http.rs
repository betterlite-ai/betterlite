use betterlite_models::decisions::Json;

use crate::gemini_embedding_records::input::ResolvedFile;

use super::error::GeminiEmbeddingCallError;

// the same cap the pooled client puts on a buffered reply
pub const EMBEDDING_BODY_LIMIT: usize = 16 * 1024 * 1024;

pub fn decode_embedding_http(bytes: &[u8]) -> Result<Json, GeminiEmbeddingCallError> {
    if bytes.len() > EMBEDDING_BODY_LIMIT {
        return Err(GeminiEmbeddingCallError::BodyTooLarge {
            limit: EMBEDDING_BODY_LIMIT,
        });
    }
    serde_json::from_slice(bytes).map_err(GeminiEmbeddingCallError::Decode)
}

// A missing mimeType becomes empty and a missing uri falls back to the file identifier.
pub fn file_metadata(file: &str, reply: &Json) -> Result<ResolvedFile, GeminiEmbeddingCallError> {
    let Json::Object(fields) = reply else {
        return Err(GeminiEmbeddingCallError::MetadataShape {
            file: file.to_owned(),
        });
    };
    let text_field = |field: &'static str, default: &str| match fields.get(field) {
        None | Some(Json::Null) => Ok(default.to_owned()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GeminiEmbeddingCallError::MetadataField {
                file: file.to_owned(),
                field,
            })
        }
    };
    Ok(ResolvedFile {
        mime_type: text_field("mimeType", "")?,
        uri: text_field("uri", file)?,
    })
}
