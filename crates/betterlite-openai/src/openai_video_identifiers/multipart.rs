use std::io::{Read, Seek, SeekFrom};

use betterlite_tokenizer::image::header::{ImageType, image_type};

use crate::api::file_type_records::{FileType, mime_from_type};

const DEFAULT_IMAGE_FILENAME: &str = "input_reference.png";
const DEFAULT_VIDEO_FILENAME: &str = "input_video.mp4";
const DEFAULT_IMAGE_MIME: &str = "image/png";
const DEFAULT_VIDEO_MIME: &str = "video/mp4";
const IMAGE_PROBE_BYTES: usize = 100;
const VIDEO_PROBE_BYTES: usize = 64;

pub trait ReadSeek: Read + Seek {}

impl<T: Read + Seek> ReadSeek for T {}

pub enum MediaBody<'a> {
    Bytes(&'a [u8]),
    Text(&'a str),
    Stream {
        name: Option<&'a str>,
        reader: &'a mut dyn ReadSeek,
    },
}

pub struct MultipartPart<'a> {
    pub field: String,
    pub filename: String,
    pub content_type: &'static str,
    pub body: MediaBody<'a>,
}

fn probe_prefix(reader: &mut dyn ReadSeek, limit: usize) -> Option<Vec<u8>> {
    let start = reader.stream_position().ok()?;
    reader.seek(SeekFrom::Start(0)).ok()?;
    let mut head = vec![0_u8; limit];
    let read = reader.read(&mut head).ok()?;
    reader.seek(SeekFrom::Start(start)).ok()?;
    head.truncate(read);
    Some(head)
}

fn mime_for_image(detected: Option<ImageType>) -> &'static str {
    match detected {
        Some(ImageType::Png) => mime_from_type(FileType::Png),
        Some(ImageType::Gif) => mime_from_type(FileType::Gif),
        Some(ImageType::Jpeg) => mime_from_type(FileType::Jpeg),
        Some(ImageType::Heic) => mime_from_type(FileType::Heic),
        Some(ImageType::Webp) => mime_from_type(FileType::Webp),
        None => DEFAULT_IMAGE_MIME,
    }
}

#[must_use]
pub fn image_content_type(image: &mut MediaBody<'_>) -> &'static str {
    let detected = match image {
        MediaBody::Bytes(bytes) => image_type(bytes.get(..IMAGE_PROBE_BYTES).unwrap_or(bytes)),
        MediaBody::Text(_) => None,
        MediaBody::Stream { reader, .. } => probe_prefix(&mut **reader, IMAGE_PROBE_BYTES)
            .as_deref()
            .and_then(image_type),
    };
    mime_for_image(detected)
}

#[must_use]
pub fn video_content_type(video: &mut MediaBody<'_>, filename: &str) -> &'static str {
    if let Some(guessed) = mime_guess::from_path(filename).first_raw()
        && guessed.starts_with("video/")
    {
        return guessed;
    }
    // LiteLLM probes for ftyp so the cursor moves as it does there, then answers video/mp4 either way
    if let MediaBody::Stream { reader, .. } = video {
        let _probe = probe_prefix(&mut **reader, VIDEO_PROBE_BYTES);
    }
    DEFAULT_VIDEO_MIME
}

pub fn add_image_to_files<'a>(
    files: &mut Vec<MultipartPart<'a>>,
    field_name: &str,
    mut image: MediaBody<'a>,
) {
    let content_type = image_content_type(&mut image);
    let filename = match &image {
        MediaBody::Stream {
            name: Some(name), ..
        } => (*name).to_owned(),
        _ => DEFAULT_IMAGE_FILENAME.to_owned(),
    };
    files.push(MultipartPart {
        field: field_name.to_owned(),
        filename,
        content_type,
        body: image,
    });
}

#[must_use]
pub fn video_file_tuple<'a>(mut video: MediaBody<'a>, field_name: &str) -> MultipartPart<'a> {
    let filename = match &video {
        MediaBody::Stream {
            name: Some(name), ..
        } if !name.is_empty() => (*name).to_owned(),
        _ => DEFAULT_VIDEO_FILENAME.to_owned(),
    };
    let content_type = video_content_type(&mut video, &filename);
    MultipartPart {
        field: field_name.to_owned(),
        filename,
        content_type,
        body: video,
    }
}
