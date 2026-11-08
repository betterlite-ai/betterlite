use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::arguments::json::{Json, Object};
use crate::media::inline::RemoteMediaFetcher;
use crate::media_fetch::error::ImageFetchError;

const DATA_PREFIX: &str = "data:";
const BASE64_MARKER: &str = ";base64,";
const ESCAPED_SLASH: &str = "\\/";
const HTTP_PREFIX: &str = "http";
const HTTP_SCHEME: &str = "http://";
const HTTPS_SCHEME: &str = "https://";
const TYPE: &str = "type";
const SOURCE: &str = "source";
const URL: &str = "url";
const FORMAT: &str = "format";
const MEDIA_TYPE: &str = "media_type";
const DATA: &str = "data";
const IMAGE: &str = "image";
const URL_SOURCE: &str = "url";
const BASE64_SOURCE: &str = "base64";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImageChunk {
    #[serde(rename = "type")]
    pub kind: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Error)]
pub enum ImageError {
    #[error(transparent)]
    Fetch(#[from] ImageFetchError),
    #[error("image_url must be a base64 data URL such as data:image/jpeg;base64,<data>")]
    DataUrl,
    #[error("image_url must be a string or an object")]
    Input,
    #[error("image_url.{field} must be a string")]
    Field { field: &'static str },
}

// LiteLLM's convert_generic_image_chunk_to_openai_image_obj builds the data URL an OpenAI image_url carries
#[must_use]
pub fn to_openai_image_url(chunk: &ImageChunk) -> String {
    let mut url = String::from("data:");
    url.push_str(&chunk.media_type);
    url.push(';');
    url.push_str(&chunk.kind);
    url.push(',');
    url.push_str(&chunk.data);
    url
}

// Python unpacks exactly two pieces from the marker split, so a second marker anywhere is an error
fn split_data_url(url: &str) -> Option<(&str, &str)> {
    let payload = url.split(DATA_PREFIX).nth(1)?;
    let (media_type, data) = payload.split_once(BASE64_MARKER)?;
    if data.contains(BASE64_MARKER) {
        return None;
    }
    Some((media_type, data))
}

pub async fn to_anthropic_image_chunk<Source>(
    openai_image_url: &str,
    format: Option<&str>,
    fetcher: &Source,
) -> Result<ImageChunk, ImageError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let resolved = if openai_image_url.starts_with(HTTP_PREFIX) {
        fetcher.convert_url(openai_image_url).await?
    } else {
        openai_image_url.to_owned()
    };
    let Some((parsed_media_type, data)) = split_data_url(&resolved) else {
        return Err(ImageError::DataUrl);
    };
    let media_type = format.map_or_else(
        || parsed_media_type.replace(ESCAPED_SLASH, "/"),
        ToOwned::to_owned,
    );
    Ok(ImageChunk {
        kind: BASE64_SOURCE.to_owned(),
        media_type,
        data: data.to_owned(),
    })
}

pub async fn create_anthropic_image_param<Source>(
    image_url_input: &Json,
    format: Option<&str>,
    is_bedrock_invoke: bool,
    fetcher: &Source,
) -> Result<Object, ImageError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let mut chosen_format = format.map(ToOwned::to_owned);
    let image_url = match image_url_input {
        Json::Text(url) => url.as_str(),
        Json::Object(fields) => {
            if chosen_format.is_none() {
                chosen_format = object_format(fields)?;
            }
            match fields.get(URL) {
                None | Some(Json::Null) => "",
                Some(Json::Text(url)) => url.as_str(),
                Some(_) => return Err(ImageError::Field { field: "url" }),
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            return Err(ImageError::Input);
        }
    };
    if image_url.starts_with(HTTP_SCHEME) || image_url.starts_with(HTTPS_SCHEME) {
        // Bedrock invoke and plain HTTP cannot carry a URL source, so both download the image instead
        if is_bedrock_invoke || image_url.starts_with(HTTP_SCHEME) {
            let base64_url = fetcher.convert_url(image_url).await?;
            let chunk =
                to_anthropic_image_chunk(&base64_url, chosen_format.as_deref(), fetcher).await?;
            return Ok(image_param_block(&ImageSource::Base64(&chunk)));
        }
        return Ok(image_param_block(&ImageSource::Url(image_url)));
    }
    let chunk = to_anthropic_image_chunk(image_url, chosen_format.as_deref(), fetcher).await?;
    Ok(image_param_block(&ImageSource::Base64(&chunk)))
}

fn object_format(fields: &Object) -> Result<Option<String>, ImageError> {
    match fields.get(FORMAT) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ImageError::Field { field: "format" }),
    }
}

enum ImageSource<'chunk> {
    Base64(&'chunk ImageChunk),
    Url(&'chunk str),
}

fn image_param_block(source: &ImageSource<'_>) -> Object {
    let element = match source {
        ImageSource::Base64(chunk) => Object::from_iter([
            (TYPE.to_owned(), Json::Text(BASE64_SOURCE.to_owned())),
            (MEDIA_TYPE.to_owned(), Json::Text(chunk.media_type.clone())),
            (DATA.to_owned(), Json::Text(chunk.data.clone())),
        ]),
        ImageSource::Url(url) => Object::from_iter([
            (TYPE.to_owned(), Json::Text(URL_SOURCE.to_owned())),
            (URL.to_owned(), Json::Text((*url).to_owned())),
        ]),
    };
    Object::from_iter([
        (TYPE.to_owned(), Json::Text(IMAGE.to_owned())),
        (SOURCE.to_owned(), Json::Object(element)),
    ])
}
