use std::collections::{BTreeMap, HashSet};
use std::mem::take;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::inline::{
    MediaPredicate, RemoteMediaFetcher, async_inline_remote_media, inline_remote_media,
};
use betterlite_prompts::media::limits::MAX_REMOTE_URLS;
use betterlite_prompts::media::parts::RemoteMedia;
use betterlite_prompts::message_content::{Content, Message};

use crate::gemini_history::content::{FORMAT, object};
use crate::gemini_history::error::HistoryError;
use crate::gemini_history::media_source::{generated_image, hints_of, media_locator};
use crate::gemini_media::error::MediaError as GeminiMediaError;
use crate::gemini_media::gcs_metadata::{AuthValues, GcsDeps, auth_values, object_content_type};
use crate::gemini_media::mime::normalize_mime_type;
use crate::gemini_media::policy::{GoogleProvider, inlines_remote};
use crate::gemini_media::process::splitext_extension;
use crate::model_policy::capabilities::VertexCapabilityProvider;

const IMAGES: &str = "images";
const IMAGE_URL: &str = "image_url";
const URL: &str = "url";
const FILE: &str = "file";
const USER: &str = "user";
const SYSTEM: &str = "system";
const ASSISTANT: &str = "assistant";
const GS_SCHEME: &str = "gs://";

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

// LiteLLM reads each extension-less gs:// object while converting its part, so this pass runs before the walkers and writes the MIME it read
pub async fn resolve_gcs_content_types(
    mut messages: Vec<Message>,
    litellm_params: &Object,
    deps: &GcsDeps<'_>,
) -> Result<Vec<Message>, HistoryError> {
    let auth = auth_values(litellm_params);
    for message in &mut messages {
        match message.role.as_str() {
            USER | SYSTEM => resolve_part_media(message, auth, deps).await?,
            ASSISTANT => resolve_image_media(message, auth, deps).await?,
            _ => {}
        }
    }
    Ok(messages)
}

async fn resolve_part_media(
    message: &mut Message,
    auth: AuthValues<'_>,
    deps: &GcsDeps<'_>,
) -> Result<(), HistoryError> {
    let Some(Content::Parts(items)) = message.content.as_mut() else {
        return Ok(());
    };
    for item in items {
        let kind = item.kind.clone();
        let Some((locator, _field)) = media_locator(item)? else {
            continue;
        };
        let (url, fields) = &locator;
        let Some(mime_type) = gcs_content_type(url, fields, auth, deps).await? else {
            continue;
        };
        attach_format(&mut item.fields, &kind, mime_type)?;
    }
    Ok(())
}

async fn resolve_image_media(
    message: &mut Message,
    auth: AuthValues<'_>,
    deps: &GcsDeps<'_>,
) -> Result<(), HistoryError> {
    let Some(Json::List(entries)) = message.provider_specific_fields.get_mut(IMAGES) else {
        return Ok(());
    };
    for entry in entries {
        let Some((url, fields)) = generated_image(entry) else {
            continue;
        };
        if let Some(mime_type) = gcs_content_type(&url, &fields, auth, deps).await?
            && let Json::Object(entry_fields) = entry
            && let Some(Json::Object(image)) = entry_fields.get_mut(IMAGE_URL)
        {
            image.insert(FORMAT.to_owned(), Json::Text(mime_type));
        }
    }
    Ok(())
}

async fn gcs_content_type(
    url: &str,
    fields: &Object,
    auth: AuthValues<'_>,
    deps: &GcsDeps<'_>,
) -> Result<Option<String>, HistoryError> {
    let probes = url.contains(GS_SCHEME)
        && hints_of(fields).explicit().is_none()
        && splitext_extension(url).is_empty();
    if !probes {
        return Ok(None);
    }
    match object_content_type(url, auth, deps).await {
        Ok(Some(content_type)) => Ok(Some(normalize_mime_type(&content_type)?)),
        Ok(None) | Err(_) => Err(HistoryError::Media(
            GeminiMediaError::UnknownGcsContentType(url.to_owned()),
        )),
    }
}

// both lookups answer the one bodyless 400 the caller sees, so the partition decides only where the failure is raised
fn attach_format(fields: &mut Object, kind: &str, mime_type: String) -> Result<(), HistoryError> {
    match kind {
        IMAGE_URL => attach_image_format(fields, mime_type),
        FILE => attach_file_format(fields, mime_type),
        // an input_audio part always carries its audio format, so it never reaches a metadata read
        _ => Ok(()),
    }
}

fn attach_image_format(fields: &mut Object, mime_type: String) -> Result<(), HistoryError> {
    let format = Json::Text(mime_type);
    let Some(value) = fields.get_mut(IMAGE_URL) else {
        return Err(HistoryError::Invalid("image_url is missing".to_owned()));
    };
    match value {
        Json::Text(text) => {
            let url = take(text);
            *value = Json::Object(object!(URL => Json::Text(url), FORMAT => format));
            Ok(())
        }
        Json::Object(image) => {
            image.insert(FORMAT.to_owned(), format);
            Ok(())
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => Err(HistoryError::Invalid(
            "image_url must be a string or an object".to_owned(),
        )),
    }
}

fn attach_file_format(fields: &mut Object, mime_type: String) -> Result<(), HistoryError> {
    let Some(Json::Object(file)) = fields.get_mut(FILE) else {
        return Err(HistoryError::Invalid("file is missing".to_owned()));
    };
    file.insert(FORMAT.to_owned(), Json::Text(mime_type));
    Ok(())
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
