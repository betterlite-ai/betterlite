use crate::gemini_media::GEMINI_FILES_API_URI_PREFIX;
use crate::gemini_media::error::{MediaError, MediaField};
use crate::gemini_media::mime::{MimeHints, apply_mime_alias, normalize_mime_type};
use crate::gemini_media::part::{Blob, FileData, MediaPart, MediaResolution, VideoMetadata};
use crate::model_urls::model_name::is_gemini_3_or_newer;
use betterlite_openai::api::file_type_records::{find_type, gemini_accepts, mime_from_type};
use betterlite_prompts::media_metadata::url_mime::image_mime_type_from_url;
const EMPTY_PART: MediaPart = MediaPart {
    inline_data: None,
    file_data: None,
    media_resolution: None,
    video_metadata: None,
};
pub struct MediaRequest<'request> {
    pub url: &'request str,
    pub hints: &'request MimeHints,
    pub resolution: Option<MediaResolution>,
    pub video_metadata: Option<&'request VideoMetadata>,
    pub model: Option<&'request str>,
    pub field: MediaField,
}
pub fn process_media(request: &MediaRequest<'_>) -> Result<MediaPart, MediaError> {
    let mut part = payload_for(request)?;
    apply_metadata(&mut part, request);
    Ok(part)
}
fn payload_for(request: &MediaRequest<'_>) -> Result<MediaPart, MediaError> {
    let url = request.url;
    if url.contains("gs://") {
        return gcs_payload(request);
    }
    if let Some(file_data) = uri_payload(request) {
        return Ok(MediaPart {
            file_data: Some(file_data),
            ..EMPTY_PART
        });
    }
    if url.contains("http://") || url.contains("https://") {
        return Err(MediaError::RemoteMediaRequired(request.field));
    }
    if url.contains("base64") {
        let (data, mime_type) = inline_payload(url, request.hints.explicit())?;
        return Ok(MediaPart {
            inline_data: Some(Blob { data, mime_type }),
            ..EMPTY_PART
        });
    }
    Err(MediaError::InvalidMedia(url.to_owned()))
}
fn uri_payload(request: &MediaRequest<'_>) -> Option<FileData> {
    let url = request.url;
    let explicit = request.hints.explicit().map(str::to_owned);
    if url.starts_with(GEMINI_FILES_API_URI_PREFIX) {
        return Some(FileData {
            mime_type: explicit,
            file_uri: url.to_owned(),
        });
    }
    if !url.contains("https://") {
        return None;
    }
    let mime_type = explicit.or_else(|| image_mime_type_from_url(url).map(str::to_owned))?;
    Some(FileData {
        mime_type: Some(mime_type),
        file_uri: url.to_owned(),
    })
}
fn gcs_payload(request: &MediaRequest<'_>) -> Result<MediaPart, MediaError> {
    let url = request.url;
    let mime_type = match request.hints.explicit() {
        Some(format) => apply_mime_alias(format),
        None => inferred_gcs_mime(url, request.field)?,
    };
    Ok(MediaPart {
        file_data: Some(FileData {
            mime_type: Some(mime_type),
            file_uri: url.to_owned(),
        }),
        ..EMPTY_PART
    })
}
fn inferred_gcs_mime(url: &str, field: MediaField) -> Result<String, MediaError> {
    let extension = splitext_extension(url);
    if extension.is_empty() {
        return Err(MediaError::RemoteMediaRequired(field));
    }
    let Some(file_type) = find_type(extension) else {
        return Err(MediaError::UnsupportedMediaType(extension.to_owned(), None));
    };
    if !gemini_accepts(file_type) {
        return Err(MediaError::UnsupportedMediaType(
            mime_from_type(file_type).to_owned(),
            None,
        ));
    }
    normalize_mime_type(mime_from_type(file_type))
}
// os.path.splitext: a dot only starts an extension when some non-dot byte precedes it in the segment
fn splitext_extension(url: &str) -> &str {
    let segment_start = url.rfind('/').map_or(0, |slash| slash.saturating_add(1));
    let segment = url.get(segment_start..).unwrap_or_default();
    let Some(dot) = segment.rfind('.') else {
        return "";
    };
    let prefix = segment.get(..dot).unwrap_or_default();
    if prefix.is_empty() || prefix.bytes().all(|byte| byte == b'.') {
        return "";
    }
    segment.get(dot.saturating_add(1)..).unwrap_or_default()
}
fn inline_payload(url: &str, format: Option<&str>) -> Result<(String, String), MediaError> {
    let Some(after_data) = url.split("data:").nth(1) else {
        return Err(MediaError::InvalidInlineData(url.to_owned()));
    };
    let mut pieces = after_data.split(";base64,");
    let (Some(media_type), Some(data), None) = (pieces.next(), pieces.next(), pieces.next()) else {
        return Err(MediaError::InvalidInlineData(url.to_owned()));
    };
    let mime_type = format.map_or_else(|| media_type.replace("\\/", "/"), str::to_owned);
    Ok((data.to_owned(), mime_type))
}
fn apply_metadata(part: &mut MediaPart, request: &MediaRequest<'_>) {
    let Some(model) = request.model else {
        return;
    };
    if is_gemini_3_or_newer(model) {
        part.media_resolution = request.resolution;
    }
    let Some(metadata) = request.video_metadata else {
        return;
    };
    if metadata.fps.is_some() || metadata.start_offset.is_some() || metadata.end_offset.is_some() {
        part.video_metadata = Some(metadata.clone());
    }
}
