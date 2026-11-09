use crate::bedrock_images::error::BedrockImageError;

pub const SUPPORTED_IMAGE_FORMATS: [&str; 4] = ["png", "jpeg", "gif", "webp"];
pub const SUPPORTED_DOCUMENT_FORMATS: [&str; 9] = [
    "pdf", "csv", "doc", "docx", "xls", "xlsx", "html", "txt", "md",
];
pub const SUPPORTED_VIDEO_FORMATS: [&str; 9] = [
    "mp4", "mov", "mkv", "webm", "flv", "mpeg", "mpg", "wmv", "3gp",
];

const IMAGE_AND_VIDEO_FORMATS_TEXT: &str = "['mp4', 'mov', 'mkv', 'webm', 'flv', 'mpeg', 'mpg', 'wmv', '3gp', 'png', 'jpeg', 'gif', 'webp']";
const DOCUMENT_FORMATS_TEXT: &str =
    "['pdf', 'csv', 'doc', 'docx', 'xls', 'xlsx', 'html', 'txt', 'md']";

// The first extension mimetypes maps to each MIME type that names a supported document format
const DOCUMENT_EXTENSIONS: &[(&str, &str)] = &[
    ("application/msword", "doc"),
    ("application/pdf", "pdf"),
    ("application/vnd.ms-excel", "xls"),
    ("text/csv", "csv"),
    ("text/html", "html"),
    ("text/markdown", "md"),
    ("text/plain", "txt"),
];

// litellm.types.files FILE_MIME_TYPES: the first extension of the file type, for MIME types mimetypes does not know
const FALLBACK_EXTENSIONS: &[(&str, &str)] = &[
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "docx",
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xlsx",
    ),
];

pub fn validate_format(mime_type: &str, image_format: &str) -> Result<String, BedrockImageError> {
    if mime_type.starts_with("application") || mime_type.starts_with("text") {
        return document_format(mime_type);
    }
    if SUPPORTED_VIDEO_FORMATS.contains(&image_format)
        || SUPPORTED_IMAGE_FORMATS.contains(&image_format)
    {
        return Ok(image_format.to_owned());
    }
    Err(BedrockImageError::UnsupportedImageFormat {
        image_format: image_format.to_owned(),
        supported: IMAGE_AND_VIDEO_FORMATS_TEXT,
    })
}

// mimetypes.guess_all_extensions(mime_type, strict=False) filtered by the supported formats, then get_file_extension_from_mime_type
fn document_format(mime_type: &str) -> Result<String, BedrockImageError> {
    let lowered = mime_type.to_lowercase();
    let known = DOCUMENT_EXTENSIONS
        .iter()
        .chain(FALLBACK_EXTENSIONS)
        .find(|(mime, _)| *mime == lowered);
    match known {
        Some((_, extension)) => Ok((*extension).to_owned()),
        None => Err(BedrockImageError::NoSupportedExtensions {
            mime_type: mime_type.to_owned(),
            supported: DOCUMENT_FORMATS_TEXT,
        }),
    }
}
