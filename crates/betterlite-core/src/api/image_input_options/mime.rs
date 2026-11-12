use std::io::{Read, Seek, SeekFrom};

use betterlite_openai::api::file_type_records::{FileType, mime_from_type};
use betterlite_tokenizer::image::header::{ImageType, image_type};

pub const SNIFF_BYTES: usize = 100;
pub const DEFAULT_MIME: &str = "image/png";

#[must_use]
pub fn mime_for_image_type(detected: Option<ImageType>) -> &'static str {
    match detected {
        Some(ImageType::Png) => mime_from_type(FileType::Png),
        Some(ImageType::Gif) => mime_from_type(FileType::Gif),
        Some(ImageType::Jpeg) => mime_from_type(FileType::Jpeg),
        Some(ImageType::Heic) => mime_from_type(FileType::Heic),
        Some(ImageType::Webp) => mime_from_type(FileType::Webp),
        None => DEFAULT_MIME,
    }
}

#[must_use]
pub fn image_type_from_name(name: &str) -> Option<ImageType> {
    match name {
        "png" => Some(ImageType::Png),
        "gif" => Some(ImageType::Gif),
        "jpeg" => Some(ImageType::Jpeg),
        "heic" => Some(ImageType::Heic),
        "webp" => Some(ImageType::Webp),
        _ => None,
    }
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
    mime_for_image_type(image_type(prefix))
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
    mime_for_image_type(image_type(head.get(..read).unwrap_or_default()))
}
