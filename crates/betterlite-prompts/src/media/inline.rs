use std::collections::BTreeMap;
use std::future::Future;

use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::{Json, Object};
use crate::media::data_url::split_base64_marker;
use crate::media::error::MediaError;
use crate::media::parts::{PartShape, RemoteMedia, parse_remote_part};
use crate::media_fetch::error::ImageFetchError;
use crate::media_fetch::fetcher::Fetcher;
use crate::message_content::{Content, ContentPart, Message};

const IMAGE_URL: &str = "image_url";
const FILE: &str = "file";
const SOURCE: &str = "source";
const FILE_ID: &str = "file_id";
const FILE_DATA: &str = "file_data";
const FORMAT: &str = "format";
const TYPE: &str = "type";
const URL: &str = "url";
const BASE64_TYPE: &str = "base64";
const PDF_MEDIA_TYPE: &str = "application/pdf";

pub type MediaPredicate = dyn Fn(&RemoteMedia) -> bool + Send + Sync;

// The seam B5083's Fetcher fills; tests and the oracle helper answer from a supplied URL mapping
pub trait RemoteMediaFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>>;

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>>;
}

impl<R> RemoteMediaFetcher for Fetcher<R>
where
    R: Resolve,
{
    // Self:: names the inherent fetcher methods, which win over these same-named trait methods
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        Self::convert(self, url)
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        Self::fetch_all(self, urls)
    }
}

#[must_use]
pub fn remote_urls_to_inline(messages: &[Message], should_inline: &MediaPredicate) -> Vec<String> {
    let mut urls = Vec::new();
    for message in messages {
        for part in content_parts(message) {
            let Some(remote) = parse_remote_part(part) else {
                continue;
            };
            if should_inline(&remote.media) && !urls.contains(&remote.media.url) {
                urls.push(remote.media.url.clone());
            }
        }
    }
    urls
}

// One convert at a time, in first-seen URL order, the way LiteLLM builds its dict at L321
pub async fn inline_remote_media<Source>(
    messages: &[Message],
    should_inline: &MediaPredicate,
    fetcher: &Source,
) -> Result<Vec<Message>, MediaError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let mut data_urls = BTreeMap::new();
    for url in remote_urls_to_inline(messages, should_inline) {
        let data_url = fetcher.convert_url(&url).await?;
        data_urls.insert(url, data_url);
    }
    inline_messages(messages, &data_urls, should_inline)
}

// B5083's fetch_all runs every URL with at most 20 in flight and answers them in input order
pub async fn async_inline_remote_media<Source>(
    messages: &[Message],
    should_inline: &MediaPredicate,
    fetcher: &Source,
) -> Result<Vec<Message>, MediaError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let urls = remote_urls_to_inline(messages, should_inline);
    if urls.is_empty() {
        return Ok(messages.to_vec());
    }
    let values = fetcher.convert_urls(&urls).await?;
    let data_urls = urls.into_iter().zip(values).collect();
    inline_messages(messages, &data_urls, should_inline)
}

pub(crate) fn inline_messages(
    messages: &[Message],
    data_urls: &BTreeMap<String, String>,
    should_inline: &MediaPredicate,
) -> Result<Vec<Message>, MediaError> {
    let mut inlined = Vec::with_capacity(messages.len());
    for message in messages {
        inlined.push(inline_message(message, data_urls, should_inline)?);
    }
    Ok(inlined)
}

fn inline_message(
    message: &Message,
    data_urls: &BTreeMap<String, String>,
    should_inline: &MediaPredicate,
) -> Result<Message, MediaError> {
    let Some(Content::Parts(parts)) = &message.content else {
        return Ok(message.clone());
    };
    if parts.is_empty() {
        return Ok(message.clone());
    }
    let mut inlined_parts = Vec::with_capacity(parts.len());
    for part in parts {
        inlined_parts.push(inline_part(part, data_urls, should_inline)?);
    }
    let mut inlined = message.clone();
    inlined.content = Some(Content::Parts(inlined_parts));
    Ok(inlined)
}

fn inline_part(
    part: &ContentPart,
    data_urls: &BTreeMap<String, String>,
    should_inline: &MediaPredicate,
) -> Result<ContentPart, MediaError> {
    let Some(remote) = parse_remote_part(part) else {
        return Ok(part.clone());
    };
    if !should_inline(&remote.media) {
        return Ok(part.clone());
    }
    let Some(data_url) = data_urls.get(&remote.media.url) else {
        return Ok(part.clone());
    };
    inline_remote_part(part, &remote.shape, &remote.media.url, data_url)
}

fn inline_remote_part(
    part: &ContentPart,
    shape: &PartShape<'_>,
    url: &str,
    data_url: &str,
) -> Result<ContentPart, MediaError> {
    let mut inlined = part.clone();
    match shape {
        PartShape::Image { image_url } => {
            let replacement = image_url.as_ref().map_or_else(
                || Json::Text(data_url.to_owned()),
                |fields| {
                    let mut replacement_fields = Object::clone(fields);
                    replacement_fields.insert(URL.to_owned(), Json::Text(data_url.to_owned()));
                    Json::Object(replacement_fields)
                },
            );
            inlined.fields.insert(IMAGE_URL.to_owned(), replacement);
        }
        PartShape::File { file } => {
            let mut inlined_file = Object::clone(file);
            inlined_file.shift_remove(FILE_ID);
            if !inlined_file.contains_key(FORMAT) && url.to_lowercase().ends_with(".pdf") {
                inlined_file.insert(FORMAT.to_owned(), Json::Text(PDF_MEDIA_TYPE.to_owned()));
            }
            inlined_file.insert(FILE_DATA.to_owned(), Json::Text(data_url.to_owned()));
            inlined
                .fields
                .insert(FILE.to_owned(), Json::Object(inlined_file));
        }
        PartShape::Source => {
            inlined.fields.insert(
                SOURCE.to_owned(),
                Json::Object(base64_source(url, data_url)?),
            );
        }
    }
    Ok(inlined)
}

fn base64_source(url: &str, data_url: &str) -> Result<Object, MediaError> {
    let Some(parsed) = split_base64_marker(data_url) else {
        return Err(MediaError::MissingBase64Marker);
    };
    let media_type = if url.to_lowercase().ends_with(".pdf") {
        PDF_MEDIA_TYPE
    } else {
        parsed.media_type
    };
    Ok(Object::from_iter([
        (TYPE.to_owned(), Json::Text(BASE64_TYPE.to_owned())),
        ("media_type".to_owned(), Json::Text(media_type.to_owned())),
        ("data".to_owned(), Json::Text(parsed.data.to_owned())),
    ]))
}

fn content_parts(message: &Message) -> &[ContentPart] {
    match &message.content {
        Some(Content::Parts(parts)) => parts,
        Some(Content::Text(_)) | None => &[],
    }
}
