use std::collections::{BTreeMap, HashSet};

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::inline::{
    MediaPredicate, RemoteMediaFetcher, async_inline_remote_media, inline_remote_media,
};
use betterlite_prompts::media::limits::MAX_REMOTE_URLS;
use betterlite_prompts::media::parts::RemoteMedia;
use betterlite_prompts::message_content::Message;

use crate::gemini_history::media_source::{generated_image, hints_of};
use crate::gemini_media::policy::{GoogleProvider, inlines_remote};
use crate::model_policy::capabilities::VertexCapabilityProvider;

const IMAGES: &str = "images";
const IMAGE_URL: &str = "image_url";
const URL: &str = "url";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchMode {
    Sequential,
    Concurrent,
}

pub async fn prepare<Source>(
    messages: &[Message],
    provider: VertexCapabilityProvider,
    fetcher: &Source,
    mode: FetchMode,
) -> Result<Vec<Message>, MediaError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let google = match provider {
        VertexCapabilityProvider::Gemini => GoogleProvider::AiStudio,
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => {
            GoogleProvider::Vertex
        }
    };
    let should_inline =
        |media: &RemoteMedia| inlines_remote(google, &media.url, &hints_of(&media.fields));
    let mut prepared = match mode {
        FetchMode::Sequential => inline_remote_media(messages, &should_inline, fetcher).await?,
        FetchMode::Concurrent => {
            async_inline_remote_media(messages, &should_inline, fetcher).await?
        }
    };
    let should_inline_image = |media: &RemoteMedia| {
        inlines_remote(GoogleProvider::Vertex, &media.url, &hints_of(&media.fields))
    };
    let urls = image_urls(&prepared, &should_inline_image)?;
    let data_urls = fetch_images(urls, fetcher, mode).await?;
    rewrite_images(&mut prepared, &should_inline_image, &data_urls);
    Ok(prepared)
}

// An assistant image is processed by LiteLLM's Gemini media walker even on Studio, so it always takes the Vertex policy
fn image_urls(
    messages: &[Message],
    should_inline: &MediaPredicate<'_>,
) -> Result<Vec<String>, MediaError> {
    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    for message in messages {
        for image in images_of(message) {
            let Some((_, media)) = image_entry(image) else {
                continue;
            };
            if !should_inline(&media) || !seen.insert(media.url.clone()) {
                continue;
            }
            if urls.len() == MAX_REMOTE_URLS {
                return Err(MediaError::TooManyRemoteUrls {
                    limit: MAX_REMOTE_URLS,
                });
            }
            urls.push(media.url);
        }
    }
    Ok(urls)
}

fn images_of(message: &Message) -> &[Json] {
    let Some(Json::List(items)) = message.provider_specific_fields.get(IMAGES) else {
        return &[];
    };
    items
}

fn image_entry(image: &Json) -> Option<(&Object, RemoteMedia)> {
    let entry = image.as_object()?;
    let (url, fields) = generated_image(image)?;
    Some((
        entry,
        RemoteMedia {
            url,
            fields,
            part_type: IMAGE_URL.to_owned(),
        },
    ))
}

async fn fetch_images<Source>(
    urls: Vec<String>,
    fetcher: &Source,
    mode: FetchMode,
) -> Result<BTreeMap<String, String>, MediaError>
where
    Source: RemoteMediaFetcher + Sync,
{
    if urls.is_empty() {
        return Ok(BTreeMap::new());
    }
    let values = match mode {
        FetchMode::Sequential => {
            let mut values = Vec::with_capacity(urls.len());
            for url in &urls {
                values.push(fetcher.convert_url(url).await?);
            }
            values
        }
        FetchMode::Concurrent => fetcher.convert_urls(&urls).await?,
    };
    Ok(urls.into_iter().zip(values).collect())
}

fn rewrite_images(
    messages: &mut [Message],
    should_inline: &MediaPredicate<'_>,
    data_urls: &BTreeMap<String, String>,
) {
    for message in messages {
        let Some(Json::List(entries)) = message.provider_specific_fields.get_mut(IMAGES) else {
            continue;
        };
        for entry in entries {
            let rewritten = rewritten_image(entry, should_inline, data_urls);
            *entry = rewritten;
        }
    }
}

// Two entries can name one url; only the entries the predicate accepted were fetched, so a hinted sibling keeps its reference
fn rewritten_image(
    entry: &Json,
    should_inline: &MediaPredicate<'_>,
    data_urls: &BTreeMap<String, String>,
) -> Json {
    let Some((fields, media)) = image_entry(entry) else {
        return entry.clone();
    };
    if !should_inline(&media) {
        return entry.clone();
    }
    let Some(data_url) = data_urls.get(&media.url) else {
        return entry.clone();
    };
    let mut image_url = media.fields;
    image_url.insert(URL.to_owned(), Json::Text(data_url.clone()));
    let mut rewritten = fields.clone();
    rewritten.insert(IMAGE_URL.to_owned(), Json::Object(image_url));
    Json::Object(rewritten)
}
