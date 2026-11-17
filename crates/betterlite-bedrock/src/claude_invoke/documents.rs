use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media::limits::MAX_REMOTE_URLS;
use betterlite_prompts::media_fetch::error::ImageFetchError;

const BASE64_MARKER: &str = ";base64,";
const BASE64_TYPE: &str = "base64";
const CONTENT: &str = "content";
const DATA: &str = "data";
const DATA_PREFIX: &str = "data:";
const DOCUMENT: &str = "document";
const MEDIA_TYPE: &str = "media_type";
const MESSAGES: &str = "messages";
const PDF_MEDIA_TYPE: &str = "application/pdf";
const SOURCE: &str = "source";
const TYPE: &str = "type";
const URL: &str = "url";
const URL_TYPE: &str = "url";

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error(transparent)]
    Fetch(#[from] ImageFetchError),
    #[error(transparent)]
    Media(#[from] MediaError),
    #[error("the value fetched for {url} is not a data URL with a base64 payload")]
    MissingMarker { url: String },
}

impl DocumentError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Fetch(_) | Self::Media(_) => Some(400),
            Self::MissingMarker { .. } => None,
        }
    }
}

// Bedrock Invoke refuses document URL sources, so every one is fetched and rewritten in place.
pub async fn convert_document_url_sources<Fetcher>(
    body: &mut Object,
    fetcher: &Fetcher,
) -> Result<(), DocumentError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    let Some(Json::List(messages)) = body.get_mut(MESSAGES) else {
        return Ok(());
    };
    check_remote_urls(messages)?;
    for message in messages.iter_mut() {
        let Json::Object(message_fields) = message else {
            continue;
        };
        let Some(Json::List(blocks)) = message_fields.get_mut(CONTENT) else {
            continue;
        };
        for block in blocks.iter_mut() {
            let Json::Object(block_fields) = block else {
                continue;
            };
            let Some(url) = document_url(block_fields).map(str::to_owned) else {
                continue;
            };
            let data_url = fetch_url(fetcher, &url).await?;
            let source = base64_source(&url, &data_url)?;
            block_fields.insert(SOURCE.to_owned(), Json::Object(source));
        }
    }
    Ok(())
}

// LiteLLM downloads every document URL it finds, so a request could keep the gateway fetching without end
fn check_remote_urls(messages: &[Json]) -> Result<(), DocumentError> {
    let mut seen = BTreeSet::new();
    for message in messages {
        let Some(Json::Object(message_fields)) = message else {
            continue;
        };
        let Some(Json::List(blocks)) = message_fields.get(CONTENT) else {
            continue;
        };
        for block in blocks {
            let Some(Json::Object(block_fields)) = block else {
                continue;
            };
            if let Some(url) = document_url(block_fields)
                && seen.insert(url)
                && seen.len() > MAX_REMOTE_URLS
            {
                return Err(DocumentError::Media(MediaError::TooManyRemoteUrls {
                    limit: MAX_REMOTE_URLS,
                }));
            }
        }
    }
    Ok(())
}

fn document_url(fields: &Object) -> Option<&str> {
    if fields.get(TYPE).and_then(Json::as_str) != Some(DOCUMENT) {
        return None;
    }
    let source = fields.get(SOURCE)?.as_object()?;
    if source.get(TYPE).and_then(Json::as_str) != Some(URL_TYPE) {
        return None;
    }
    source.get(URL)?.as_str()
}

async fn fetch_url<Fetcher>(fetcher: &Fetcher, url: &str) -> Result<String, DocumentError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    // LiteLLM's convert_url_to_base64 hands an already base64 data URL straight back.
    if url.starts_with(DATA_PREFIX) && url.contains(BASE64_MARKER) {
        return Ok(url.to_owned());
    }
    Ok(fetcher.convert_url(url).await?)
}

// A URL ending in .pdf names application/pdf; every other URL keeps the fetched media type.
fn base64_source(url: &str, data_url: &str) -> Result<Object, DocumentError> {
    let Some(body) = data_url.strip_prefix(DATA_PREFIX) else {
        return Err(DocumentError::MissingMarker {
            url: url.to_owned(),
        });
    };
    let Some((fetched_type, data)) = body.split_once(BASE64_MARKER) else {
        return Err(DocumentError::MissingMarker {
            url: url.to_owned(),
        });
    };
    let media_type = if url.to_lowercase().ends_with(".pdf") {
        PDF_MEDIA_TYPE
    } else {
        fetched_type
    };
    Ok(Object::from_iter([
        (TYPE.to_owned(), Json::Text(BASE64_TYPE.to_owned())),
        (MEDIA_TYPE.to_owned(), Json::Text(media_type.to_owned())),
        (DATA.to_owned(), Json::Text(data.to_owned())),
    ]))
}
