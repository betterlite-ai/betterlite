use betterlite_openai::api::file_type_records::{find_type, gemini_accepts, mime_from_type};
use betterlite_prompts::media_metadata::url_mime::image_mime_type_from_url;

use crate::gemini_media::GEMINI_FILES_API_URI_PREFIX;
use crate::gemini_media::error::{MediaError, MediaField};
use crate::gemini_media::mime::{MimeHints, apply_mime_alias, normalize_mime_type};
use crate::gemini_media::part::{Blob, FileData, MediaPart, MediaResolution, VideoMetadata};
use crate::model_urls::model_name::is_gemini_3_or_newer;

pub struct MediaRequest<'request> {
    pub url: &'request str,
    pub hints: &'request MimeHints,
    pub resolution: Option<MediaResolution>,
    pub video_metadata: Option<&'request VideoMetadata>,
    pub model: Option<&'request str>,
    pub field: MediaField,
}

enum Payload {
    Inline(Blob),
    File(FileData),
}

pub fn process_media(request: &MediaRequest<'_>) -> Result<MediaPart, MediaError> {
    let payload = payload_for(request)?;
    let mut part = match payload {
        Payload::Inline(blob) => MediaPart {
            inline_data: Some(blob),
            file_data: None,
            media_resolution: None,
            video_metadata: None,
        },
        Payload::File(data) => MediaPart {
            inline_data: None,
            file_data: Some(data),
            media_resolution: None,
            video_metadata: None,
        },
    };
    apply_metadata(&mut part, request);
    Ok(part)
}

fn payload_for(request: &MediaRequest<'_>) -> Result<Payload, MediaError> {
    let url = request.url;
    if url.contains("gs://") {
        return gcs_payload(request);
    }
    if url.starts_with(GEMINI_FILES_API_URI_PREFIX) {
        return Ok(Payload::File(FileData {
            mime_type: request.hints.explicit().map(str::to_owned),
            file_uri: url.to_owned(),
        }));
    }
    if url.contains("https://") {
        let mime_type = request
            .hints
            .explicit()
            .map(str::to_owned)
            .or_else(|| image_mime_type_from_url(url).map(str::to_owned));
        if let Some(inferred) = mime_type {
            return Ok(Payload::File(FileData {
                mime_type: Some(inferred),
                file_uri: url.to_owned(),
            }));
        }
    }
    if url.contains("http://") || url.contains("https://") {
        return Err(MediaError::RemoteMediaRequired {
            url: url.to_owned(),
            field: request.field,
        });
    }
    if url.contains("base64") {
        return inline_payload(url, request.hints.explicit());
    }
    Err(MediaError::InvalidMedia {
        value: url.to_owned(),
    })
}

fn gcs_payload(request: &MediaRequest<'_>) -> Result<Payload, MediaError> {
    let url = request.url;
    let mime_type = if let Some(format) = request.hints.explicit() {
        apply_mime_alias(format)
    } else {
        let extension = splitext_extension(url);
        if extension.is_empty() {
            return Err(MediaError::RemoteMediaRequired {
                url: url.to_owned(),
                field: request.field,
            });
        }
        let Some(file_type) = find_type(extension) else {
            return Err(MediaError::UnsupportedMediaType {
                subject: extension.to_owned(),
                source: None,
            });
        };
        if !gemini_accepts(file_type) {
            return Err(MediaError::UnsupportedMediaType {
                subject: mime_from_type(file_type).to_owned(),
                source: None,
            });
        }
        normalize_mime_type(mime_from_type(file_type))?
    };
    Ok(Payload::File(FileData {
        mime_type: Some(mime_type),
        file_uri: url.to_owned(),
    }))
}

// os.path.splitext: the last path segment decides, and an extension never starts at the segment's first byte
fn splitext_extension(url: &str) -> &str {
    let segment_start = url.rfind('/').map_or(0, |slash| slash.saturating_add(1));
    let segment = url.get(segment_start..).unwrap_or_default();
    match segment.rfind('.') {
        Some(dot) if dot > 0 => segment.get(dot.saturating_add(1)..).unwrap_or_default(),
        _ => "",
    }
}

fn inline_payload(url: &str, format: Option<&str>) -> Result<Payload, MediaError> {
    let Some(after_data) = url.split("data:").nth(1) else {
        return Err(MediaError::InvalidInlineData {
            value: url.to_owned(),
        });
    };
    let mut pieces = after_data.split(";base64,");
    let (Some(media_type), Some(data), None) = (pieces.next(), pieces.next(), pieces.next()) else {
        return Err(MediaError::InvalidInlineData {
            value: url.to_owned(),
        });
    };
    let mime_type = format.map_or_else(|| media_type.replace("\\/", "/"), str::to_owned);
    Ok(Payload::Inline(Blob {
        data: data.to_owned(),
        mime_type,
    }))
}

fn apply_metadata(part: &mut MediaPart, request: &MediaRequest<'_>) {
    let Some(model) = request.model else {
        return;
    };
    if is_gemini_3_or_newer(model) {
        part.media_resolution = request.resolution;
    }
    if let Some(metadata) = request
        .video_metadata
        .filter(|metadata| !metadata.is_empty())
    {
        part.video_metadata = Some(metadata.clone());
    }
}
