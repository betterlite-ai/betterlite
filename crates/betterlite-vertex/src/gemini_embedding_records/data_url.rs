use crate::gemini_embedding_records::error::GeminiEmbeddingError;

pub const SUPPORTED_MIME_TYPES: [&str; 7] = [
    "application/pdf",
    "audio/mpeg",
    "audio/wav",
    "image/jpeg",
    "image/png",
    "video/mp4",
    "video/quicktime",
];
const EXTENSION_MIME_TYPES: [(&str, &str); 8] = [
    (".jpeg", "image/jpeg"),
    (".jpg", "image/jpeg"),
    (".mov", "video/quicktime"),
    (".mp3", "audio/mpeg"),
    (".mp4", "video/mp4"),
    (".pdf", "application/pdf"),
    (".png", "image/png"),
    (".wav", "audio/wav"),
];
#[derive(Debug)]
pub struct EmbeddingDataUrl<'url> {
    pub mime_type: &'url str,
    pub payload: &'url str,
}
pub fn decode_embedding_data_url(
    data_url: &str,
) -> Result<EmbeddingDataUrl<'_>, GeminiEmbeddingError> {
    let Some(remainder) = data_url.strip_prefix("data:") else {
        return Err(GeminiEmbeddingError::Rejected {
            reason: String::from("the data URL does not start with data:"),
        });
    };
    let Some((metadata, payload)) = remainder.split_once(',') else {
        return Err(GeminiEmbeddingError::Rejected {
            reason: String::from("the data URL has no comma before its payload"),
        });
    };
    let mime_type = metadata.split_once(';').map_or(metadata, |(head, _)| head);
    if !SUPPORTED_MIME_TYPES.contains(&mime_type) {
        return Err(GeminiEmbeddingError::Rejected {
            reason: format!("{mime_type} is not a supported embedding media type"),
        });
    }
    Ok(EmbeddingDataUrl { mime_type, payload })
}
pub fn infer_mime_from_gcs_url(gcs_url: &str) -> Result<&'static str, GeminiEmbeddingError> {
    let lowered = gcs_url.to_lowercase();
    for (extension, mime_type) in EXTENSION_MIME_TYPES {
        if lowered.ends_with(extension) {
            return Ok(mime_type);
        }
    }
    Err(GeminiEmbeddingError::Rejected {
        reason: format!("no supported embedding media type matches the extension of {gcs_url}"),
    })
}
