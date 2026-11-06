use crate::arguments::json::{Json, Object};
use crate::message_content::ContentPart;

const IMAGE_URL: &str = "image_url";
const FILE: &str = "file";
const SOURCE: &str = "source";
const FILE_ID: &str = "file_id";
const TYPE: &str = "type";
const URL: &str = "url";
const URL_TYPE: &str = "url";
const REMOTE_PREFIXES: [&str; 2] = ["http://", "https://"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteMedia {
    pub url: String,
    pub fields: Object,
    pub part_type: String,
}

pub const INLINE_EVERY_REMOTE_URL: fn(&RemoteMedia) -> bool = |_media| true;

pub const INLINE_REMOTE_IMAGE_URLS: fn(&RemoteMedia) -> bool = |media| media.part_type == IMAGE_URL;

pub(crate) struct RemotePart<'part> {
    pub(crate) media: RemoteMedia,
    pub(crate) shape: PartShape<'part>,
}

pub(crate) enum PartShape<'part> {
    Image { image_url: Option<&'part Object> },
    File { file: &'part Object },
    Source,
}

pub(crate) fn parse_remote_part(part: &ContentPart) -> Option<RemotePart<'_>> {
    parse_remote_image(part)
        .or_else(|| parse_remote_file(part))
        .or_else(|| parse_remote_source(part))
}

fn parse_remote_image(part: &ContentPart) -> Option<RemotePart<'_>> {
    if part.kind != IMAGE_URL {
        return None;
    }
    let image_url = part.fields.get(IMAGE_URL);
    let image_url_fields = image_url.and_then(Json::as_object);
    let candidate = image_url_fields.map_or(image_url, |fields| fields.get(URL));
    let url = remote_url(candidate)?;
    Some(RemotePart {
        media: RemoteMedia {
            url: url.to_owned(),
            fields: image_url_fields.cloned().unwrap_or_default(),
            part_type: IMAGE_URL.to_owned(),
        },
        shape: PartShape::Image {
            image_url: image_url_fields,
        },
    })
}

fn parse_remote_file(part: &ContentPart) -> Option<RemotePart<'_>> {
    if part.kind != FILE {
        return None;
    }
    let file = part.fields.get(FILE).and_then(Json::as_object)?;
    let url = remote_url(file.get(FILE_ID))?;
    Some(RemotePart {
        media: RemoteMedia {
            url: url.to_owned(),
            fields: file.clone(),
            part_type: FILE.to_owned(),
        },
        shape: PartShape::File { file },
    })
}

fn parse_remote_source(part: &ContentPart) -> Option<RemotePart<'_>> {
    if !matches!(part.kind.as_str(), "document" | "image") {
        return None;
    }
    let source = part.fields.get(SOURCE).and_then(Json::as_object)?;
    if source.get(TYPE).and_then(Json::as_str) != Some(URL_TYPE) {
        return None;
    }
    let url = remote_url(source.get(URL))?;
    Some(RemotePart {
        media: RemoteMedia {
            url: url.to_owned(),
            fields: source.clone(),
            part_type: part.kind.clone(),
        },
        shape: PartShape::Source,
    })
}

fn remote_url(candidate: Option<&Json>) -> Option<&str> {
    let url = candidate.and_then(Json::as_str)?;
    REMOTE_PREFIXES
        .iter()
        .any(|prefix| url.starts_with(prefix))
        .then_some(url)
}
