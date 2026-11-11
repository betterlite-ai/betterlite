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

pub struct EmbeddingDataUrl<'url> {
    pub mime_type: &'url str,
    pub payload: &'url str,
}

pub fn decode_embedding_data_url(
    data_url: &str,
) -> Result<EmbeddingDataUrl<'_>, GeminiEmbeddingError> {
    let Some(remainder) = data_url.strip_prefix("data:") else {
        return Err(GeminiEmbeddingError::DataUrlPrefix);
    };
    let Some((metadata, payload)) = remainder.split_once(',') else {
        return Err(GeminiEmbeddingError::DataUrlComma);
    };
    let mime_type = match metadata.split_once(';') {
        Some((head, _parameters)) => head,
        None => metadata,
    };
    if !SUPPORTED_MIME_TYPES.contains(&mime_type) {
        return Err(GeminiEmbeddingError::UnsupportedMime {
            mime_type: mime_type.to_owned(),
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
    Err(GeminiEmbeddingError::UnsupportedExtension {
        url: gcs_url.to_owned(),
    })
}
