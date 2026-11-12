use std::io::{Read, Seek, SeekFrom};

use betterlite_openai::api::file_type_records::{FileType, mime_from_type};
use betterlite_tokenizer::image::header::{ImageType, image_type};

pub const SNIFF_BYTES: usize = 100;
pub const DEFAULT_MIME: &str = "image/png";

#[must_use]
pub fn mime_for_detected(detected: Option<&str>) -> &'static str {
    match detected {
        Some("png") => mime_from_type(FileType::Png),
        Some("jpeg") => mime_from_type(FileType::Jpeg),
        Some("gif") => mime_from_type(FileType::Gif),
        Some("webp") => mime_from_type(FileType::Webp),
        Some("heic") => mime_from_type(FileType::Heic),
        Some(_) | None => DEFAULT_MIME,
    }
}

#[must_use]
pub fn image_content_type_bytes(data: &[u8]) -> &'static str {
    let prefix = data.get(..SNIFF_BYTES).unwrap_or(data);
    let detected = image_type(prefix).map(ImageType::as_str);
    mime_for_detected(detected)
}

#[must_use]
pub fn image_content_type_seekable<R>(reader: &mut R) -> &'static str
where
    R: Read + Seek,
{
    let Ok(start) = reader.stream_position() else {
        return DEFAULT_MIME;
    };
    if reader.seek(SeekFrom::Start(0)).is_err() {
        return DEFAULT_MIME;
    }
    let mut head = [0_u8; SNIFF_BYTES];
    let Ok(read) = reader.read(&mut head) else {
        return DEFAULT_MIME;
    };
    if reader.seek(SeekFrom::Start(start)).is_err() {
        return DEFAULT_MIME;
    }
    let prefix = head.get(..read).unwrap_or_default();
    mime_for_detected(image_type(prefix).map(ImageType::as_str))
}

#[must_use]
pub fn image_content_type_stream<R>(reader: &mut R) -> &'static str
where
    R: Read,
{
    let mut head = [0_u8; SNIFF_BYTES];
    let Ok(read) = reader.read(&mut head) else {
        return DEFAULT_MIME;
    };
    mime_for_detected(image_type(head.get(..read).unwrap_or_default()).map(ImageType::as_str))
}
