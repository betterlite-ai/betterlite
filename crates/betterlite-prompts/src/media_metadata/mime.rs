const EXTENSION_TO_MIME: &[(&str, &str)] = &[
    ("aac", "audio/aac"),
    ("avi", "video/x-msvideo"),
    ("bmp", "image/bmp"),
    ("csv", "text/csv"),
    ("doc", "application/msword"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("flac", "audio/flac"),
    ("gif", "image/gif"),
    ("heic", "image/heic"),
    ("htm", "text/html"),
    ("html", "text/html"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("json", "application/json"),
    ("md", "text/markdown"),
    ("mov", "video/quicktime"),
    ("mp3", "audio/mpeg"),
    ("mp4", "video/mp4"),
    ("mpeg", "video/mpeg"),
    ("mpg", "video/mpeg"),
    ("ogg", "audio/ogg"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("ppt", "application/vnd.ms-powerpoint"),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ("svg", "image/svg+xml"),
    ("tiff", "image/tiff"),
    ("txt", "text/plain"),
    ("wav", "audio/x-wav"),
    ("webm", "video/webm"),
    ("webp", "image/webp"),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("xml", "application/xml"),
    ("zip", "application/zip"),
];

// Mirrors posixpath.splitext: a leading dot in the basename is not an extension marker.
#[must_use]
pub(super) fn guess_mime_type(filename: &str) -> Option<&'static str> {
    let basename_start = filename.rfind('/').map_or(0, |sep| sep.saturating_add(1));
    let dot = filename.rfind('.')?;
    if dot < basename_start {
        return None;
    }
    let basename_prefix = filename.get(basename_start..dot)?;
    if basename_prefix.chars().all(|character| character == '.') {
        return None;
    }
    let extension = filename.get(dot.saturating_add(1)..)?;
    if extension.is_empty() {
        return None;
    }
    let lowered = extension.to_lowercase();
    EXTENSION_TO_MIME
        .iter()
        .find(|(key, _)| *key == lowered)
        .map(|(_, value)| *value)
}
