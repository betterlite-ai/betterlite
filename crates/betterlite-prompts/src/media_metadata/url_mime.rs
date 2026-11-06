use super::error::MediaMetadataError;

// Groups mirror the Python dict literal's insertion order, since ".mpeg" appears under both video and audio.
const PATH_EXTENSION_GROUPS: &[(&[&str], &str)] = &[
    (&[".jpg", ".jpeg"], "image/jpeg"),
    (&[".png"], "image/png"),
    (&[".webp"], "image/webp"),
    (&[".mp4"], "video/mp4"),
    (&[".mov"], "video/mov"),
    (&[".mpeg", ".mpg"], "video/mpeg"),
    (&[".avi"], "video/avi"),
    (&[".wmv"], "video/wmv"),
    (&[".mpegps"], "video/mpegps"),
    (&[".flv"], "video/flv"),
    (&[".mp3"], "audio/mp3"),
    (&[".wav"], "audio/wav"),
    (&[".mpeg"], "audio/mpeg"),
    (&[".ogg"], "audio/ogg"),
    (&[".pdf"], "application/pdf"),
    (&[".txt"], "text/plain"),
];

const URL_EXTENSION_TO_MIME: &[(&str, &str)] = &[
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("pdf", "application/pdf"),
    ("csv", "text/csv"),
    ("doc", "application/msword"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("html", "text/html"),
    ("txt", "text/plain"),
    ("md", "text/markdown"),
];

const GENERIC_CONTENT_TYPES: &[&str] = &["binary/octet-stream", "application/octet-stream"];

// urllib.parse.uses_params: schemes whose last path segment may carry a ";params" suffix.
const USES_PARAMS: &[&str] = &[
    "", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtsps", "rtspu",
    "sip", "sips", "mms", "sftp", "tel",
];

// A ";params" suffix on the last path segment is not part of the path itself.
fn strip_last_segment_params(path: &str) -> &str {
    let segment_start = path.rfind('/').map_or(0, |slash| slash.saturating_add(1));
    let segment = path.get(segment_start..).unwrap_or_default();
    segment.find(';').map_or(path, |semicolon| {
        path.get(..segment_start.saturating_add(semicolon))
            .unwrap_or(path)
    })
}

// Lenient stand-in for Python's urlparse(url).path, including the uses_params trailing-segment strip.
fn url_path(url: &str) -> &str {
    let scheme_end = url.find("://");
    let after_scheme = scheme_end.map_or(url, |end| {
        url.get(end.saturating_add(3)..).unwrap_or_default()
    });
    let path = if scheme_end.is_some() {
        after_scheme
            .find(['/', '?', '#'])
            .and_then(|boundary| after_scheme.get(boundary..))
            .unwrap_or_default()
    } else {
        after_scheme
    };
    let query_or_fragment = path.find(['?', '#']).unwrap_or(path.len());
    let truncated = path.get(..query_or_fragment).unwrap_or_default();
    let scheme = scheme_end.map_or("", |scheme_len| url.get(..scheme_len).unwrap_or_default());
    if USES_PARAMS.contains(&scheme) {
        strip_last_segment_params(truncated)
    } else {
        truncated
    }
}

#[must_use]
pub fn image_mime_type_from_url(url: &str) -> Option<&'static str> {
    let lowered = url.to_lowercase();
    let path = url_path(&lowered);
    PATH_EXTENSION_GROUPS
        .iter()
        .find(|(extensions, _)| extensions.iter().any(|ext| path.ends_with(ext)))
        .map(|(_, mime)| *mime)
}

pub fn infer_content_type(
    url: &str,
    content: &[u8],
    current_content_type: Option<&str>,
) -> Result<String, MediaMetadataError> {
    if let Some(declared) = current_content_type
        && !declared.is_empty()
        && !GENERIC_CONTENT_TYPES.contains(&declared)
    {
        return Ok(declared.to_owned());
    }
    if !url.is_empty() {
        let last_segment = url.rsplit('.').next().unwrap_or(url).to_lowercase();
        let extension = last_segment.split('?').next().unwrap_or_default();
        if let Some((_, mime)) = URL_EXTENSION_TO_MIME
            .iter()
            .find(|(key, _)| *key == extension)
        {
            return Ok((*mime).to_owned());
        }
    }
    let sniff_window = content.get(..content.len().min(100)).unwrap_or(content);
    if !sniff_window.is_empty()
        && let Some(mime) = sniff_image_mime(sniff_window)
    {
        return Ok(mime.to_owned());
    }
    Err(MediaMetadataError::ContentTypeUndetermined {
        url: url.to_owned(),
    })
}

// Ported from get_image_type (token_counter.py): slice bounds never panic past a short buffer.
fn sniff_image_mime(data: &[u8]) -> Option<&'static str> {
    if data.get(0..8) == Some(&b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a"[..]) {
        return Some("image/png");
    }
    if data.get(0..4) == Some(b"GIF8") && data.get(5..6) == Some(b"a") {
        return Some("image/gif");
    }
    if data.get(0..3) == Some(&b"\xff\xd8\xff"[..]) {
        return Some("image/jpeg");
    }
    if data.get(4..8) == Some(b"ftyp") {
        return Some("image/heic");
    }
    if data.get(0..4) == Some(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        return Some("image/webp");
    }
    None
}
