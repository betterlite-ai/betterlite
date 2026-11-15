use betterlite_prompts::anthropic_invokes::documents::process_openai_file_message;
use betterlite_prompts::anthropic_results::document_uri::is_anthropic_document_data_uri;
use betterlite_prompts::anthropic_results::images::create_anthropic_image_param;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, ContentPart};

use super::{Fetcher, MessageError, part_json};

pub(super) async fn convert_image_url_blocks<Source>(
    content: Option<&Content>,
    fetcher: &Source,
) -> Result<Json, MessageError>
where
    Source: Fetcher,
{
    let Some(Content::Parts(parts)) = content else {
        return Ok(super::content_json(content));
    };
    let mut converted = Vec::with_capacity(parts.len());
    for part in parts {
        let block = if part.kind == "image_url" {
            Json::Object(convert_block(part, fetcher).await?)
        } else {
            part_json(part)
        };
        converted.push(block);
    }
    Ok(Json::List(converted))
}

async fn convert_block<Source>(part: &ContentPart, fetcher: &Source) -> Result<Object, MessageError>
where
    Source: Fetcher,
{
    let Some(image_url) = part.fields.get("image_url") else {
        return Ok(part_json(part).as_object().cloned().unwrap_or_default());
    };
    let Some(url) = image_url_url(image_url).filter(|url| !url.is_empty()) else {
        return Ok(part_json(part).as_object().cloned().unwrap_or_default());
    };
    let format = image_url
        .as_object()
        .and_then(|fields| fields.get("format"))
        .and_then(Json::as_str);
    let mut block = if is_anthropic_document_data_uri(url) {
        let element = Object::from_iter([
            ("type".to_owned(), Json::Text("file".to_owned())),
            (
                "file".to_owned(),
                Json::Object(Object::from_iter([(
                    "file_data".to_owned(),
                    Json::Text(url.to_owned()),
                )])),
            ),
        ]);
        process_openai_file_message(&element, fetcher).await?
    } else {
        create_anthropic_image_param(image_url, format, true, fetcher).await?
    };
    if let Some(cache_control) = part
        .fields
        .get("cache_control")
        .filter(|value| !matches!(value, Json::Null))
    {
        block.insert("cache_control".to_owned(), cache_control.clone());
    }
    Ok(block)
}

fn image_url_url(image_url: &Json) -> Option<&str> {
    match image_url {
        Json::Text(url) => Some(url.as_str()),
        Json::Object(fields) => fields.get("url").and_then(Json::as_str),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    }
}
