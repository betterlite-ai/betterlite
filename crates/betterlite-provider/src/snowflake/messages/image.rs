use betterlite_prompts::anthropic_invokes::documents::{
    ContentBlockType, process_openai_file_message, select_content_block_type,
};
use betterlite_prompts::anthropic_results::images::create_anthropic_image_param;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::{Content, ContentPart};

use super::{MessageError, part_json};

const IMAGE_URL: &str = "image_url";
const URL: &str = "url";
const FORMAT: &str = "format";
const CACHE_CONTROL: &str = "cache_control";
const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const DATA_PREFIX: &str = "data:";

pub async fn convert_image_url_blocks<Source>(
    content: Option<&Content>,
    fetcher: &Source,
) -> Result<Json, MessageError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let Some(Content::Parts(parts)) = content else {
        return Ok(super::content_json(content));
    };
    let mut converted = Vec::with_capacity(parts.len());
    for part in parts {
        if part.kind == IMAGE_URL {
            converted.push(Json::Object(convert_block(part, fetcher).await?));
        } else {
            converted.push(part_json(part));
        }
    }
    Ok(Json::List(converted))
}

async fn convert_block<Source>(part: &ContentPart, fetcher: &Source) -> Result<Object, MessageError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let Some(image_url) = part.fields.get(IMAGE_URL) else {
        return Ok(part_json(part).as_object().cloned().unwrap_or_default());
    };
    let url = image_url_url(image_url);
    let Some(url) = url.filter(|url| !url.is_empty()) else {
        return Ok(part_json(part).as_object().cloned().unwrap_or_default());
    };
    let format = image_url.as_object().and_then(|fields| {
        fields
            .get(FORMAT)
            .filter(|value| matches!(value, Json::Text(_)))
            .and_then(Json::as_str)
    });
    let mut block =
        if select_content_block_type(data_uri_media_type(url)) == ContentBlockType::Document {
            let element = Object::from_iter([
                ("type".to_owned(), Json::Text(FILE.to_owned())),
                (
                    FILE.to_owned(),
                    Json::Object(Object::from_iter([(
                        FILE_DATA.to_owned(),
                        Json::Text(url.to_owned()),
                    )])),
                ),
            ]);
            process_openai_file_message(&element, fetcher).await?
        } else {
            create_anthropic_image_param(image_url, format, true, fetcher).await?
        };
    if let Some(cache_control) = part.fields.get(CACHE_CONTROL)
        && !matches!(cache_control, Json::Null)
    {
        block.insert(CACHE_CONTROL.to_owned(), cache_control.clone());
    }
    Ok(block)
}

fn image_url_url(image_url: &Json) -> Option<&str> {
    match image_url {
        Json::Text(url) => Some(url.as_str()),
        Json::Object(fields) => fields.get(URL).and_then(Json::as_str),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    }
}

// Python's re.match(r"data:([^;,]+)") reads the media type up to the first ; or ,
fn data_uri_media_type(url: &str) -> &str {
    let Some(rest) = url.strip_prefix(DATA_PREFIX) else {
        return "";
    };
    let end = rest
        .find(|candidate| candidate == ';' || candidate == ',')
        .unwrap_or(rest.len());
    &rest[..end]
}
