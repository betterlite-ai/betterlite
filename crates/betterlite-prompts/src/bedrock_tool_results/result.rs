use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::Json;
use crate::bedrock_images::processor::process_image;
use crate::bedrock_images::remote::RemoteImage;
use crate::bedrock_images::types::ContentBlock;
use crate::message_content::{Content, ContentPart, Message};

use super::error::BedrockToolResultsError;
use super::ids::sanitize_bedrock_tool_use_id;
use super::types::{ToolResultBlock, ToolResultContentBlock, ToolResultPart, ToolResultStatus};

const SEARCH_RESULTS: &str = "search_results";
const TOOL_CALL_ID: &str = "tool_call_id";
const TEXT: &str = "text";
const IMAGE_URL: &str = "image_url";
const URL: &str = "url";
const FORMAT: &str = "format";
const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const FILE_ID: &str = "file_id";

pub async fn to_bedrock_tool_result<R, Ids>(
    message: &Message,
    remote: &RemoteImage<R>,
    new_id: &Ids,
) -> Result<ToolResultContentBlock, BedrockToolResultsError>
where
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let (content, used_search_results) = build_content_blocks(message, remote).await?;
    let tool_use_id = match message.provider_specific_fields.get(TOOL_CALL_ID) {
        None | Some(Json::Null) => sanitize_bedrock_tool_use_id(&new_id()),
        Some(Json::Text(tool_call_id)) => sanitize_bedrock_tool_use_id(tool_call_id),
        Some(_) => return Err(BedrockToolResultsError::ResultToolCallIdNotText),
    };
    Ok(ToolResultContentBlock {
        tool_result: ToolResultBlock {
            content,
            tool_use_id,
            status: used_search_results.then_some(ToolResultStatus::Success),
        },
    })
}

async fn build_content_blocks<R>(
    message: &Message,
    remote: &RemoteImage<R>,
) -> Result<(Vec<ToolResultPart>, bool), BedrockToolResultsError>
where
    R: Resolve,
{
    if let Some(Json::List(results)) = message.provider_specific_fields.get(SEARCH_RESULTS) {
        let mut blocks: Vec<ToolResultPart> = Vec::new();
        for result in results {
            if let Json::Object(search_result) = result {
                blocks.push(ToolResultPart::SearchResult {
                    search_result: search_result.clone(),
                });
            }
        }
        if !blocks.is_empty() {
            return Ok((blocks, true));
        }
    }
    match message.content.as_ref() {
        None => Ok((Vec::new(), false)),
        Some(Content::Text(text)) => Ok((vec![ToolResultPart::Text { text: text.clone() }], false)),
        Some(Content::Parts(parts)) => Ok((content_blocks_from_parts(parts, remote).await?, false)),
    }
}

async fn content_blocks_from_parts<R>(
    parts: &[ContentPart],
    remote: &RemoteImage<R>,
) -> Result<Vec<ToolResultPart>, BedrockToolResultsError>
where
    R: Resolve,
{
    let mut blocks: Vec<ToolResultPart> = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        match part.kind.as_str() {
            TEXT => blocks.push(ToolResultPart::Text {
                text: text_of(part, index)?,
            }),
            IMAGE_URL => append_image_url_part(&mut blocks, part, index, remote).await?,
            FILE => append_file_part(&mut blocks, part, index, remote).await?,
            _ => {}
        }
    }
    Ok(blocks)
}

fn text_of(part: &ContentPart, index: usize) -> Result<String, BedrockToolResultsError> {
    match part.fields.get(TEXT) {
        None => Err(BedrockToolResultsError::TextPartWithoutText { index }),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(BedrockToolResultsError::TextPartNotText { index }),
    }
}

async fn append_image_url_part<R>(
    blocks: &mut Vec<ToolResultPart>,
    part: &ContentPart,
    index: usize,
    remote: &RemoteImage<R>,
) -> Result<(), BedrockToolResultsError>
where
    R: Resolve,
{
    let (image_url, format) = match part.fields.get(IMAGE_URL) {
        None => return Err(BedrockToolResultsError::ImageUrlWithoutUrl { index }),
        Some(Json::Text(image_url)) => (image_url.as_str(), None),
        Some(Json::Object(fields)) => {
            let image_url = match fields.get(URL) {
                None => return Err(BedrockToolResultsError::ImageUrlWithoutAddress { index }),
                Some(Json::Text(image_url)) => image_url.as_str(),
                Some(_) => return Err(BedrockToolResultsError::ImageUrlNotText { index }),
            };
            let format = match fields.get(FORMAT) {
                None | Some(Json::Null) => None,
                Some(Json::Text(format)) => Some(format.as_str()),
                Some(_) => return Err(BedrockToolResultsError::ImageFormatNotText { index }),
            };
            (image_url, format)
        }
        Some(_) => return Err(BedrockToolResultsError::ImageUrlNotTextOrObject { index }),
    };
    let processed = process_image(image_url, format, remote)
        .await
        .map_err(|source| BedrockToolResultsError::Image { source })?;
    push_media_block(blocks, processed);
    Ok(())
}

async fn append_file_part<R>(
    blocks: &mut Vec<ToolResultPart>,
    part: &ContentPart,
    index: usize,
    remote: &RemoteImage<R>,
) -> Result<(), BedrockToolResultsError>
where
    R: Resolve,
{
    let file = match part.fields.get(FILE) {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => return Err(BedrockToolResultsError::FileNotObject { index }),
    };
    let file_data = match file.and_then(|fields| fields.get(FILE_DATA)) {
        None | Some(Json::Null) => None,
        Some(Json::Text(file_data)) => Some(file_data.as_str()),
        Some(_) => return Err(BedrockToolResultsError::FileDataNotText { index }),
    };
    let file_id = match file.and_then(|fields| fields.get(FILE_ID)) {
        None | Some(Json::Null) => None,
        Some(Json::Text(file_id)) => Some(file_id.as_str()),
        Some(_) => return Err(BedrockToolResultsError::FileIdNotText { index }),
    };
    let format = match file.and_then(|fields| fields.get(FORMAT)) {
        None | Some(Json::Null) => None,
        Some(Json::Text(format)) => Some(format.as_str()),
        Some(_) => return Err(BedrockToolResultsError::FileFormatNotText { index }),
    };
    let Some(image_url) = file_id.or(file_data) else {
        return Err(BedrockToolResultsError::FileWithoutMedia { index });
    };
    let processed = process_image(image_url, format, remote)
        .await
        .map_err(|source| BedrockToolResultsError::Image { source })?;
    push_media_block(blocks, processed);
    Ok(())
}

fn push_media_block(blocks: &mut Vec<ToolResultPart>, processed: ContentBlock) {
    match processed {
        ContentBlock::Image { image } => blocks.push(ToolResultPart::Image { image }),
        ContentBlock::Document { document } => blocks.push(ToolResultPart::Document { document }),
        // Bedrock's toolResult.content has no video member, so LiteLLM warns and drops the block
        ContentBlock::Video { video } => {
            tracing::warn!(format = %video.format, "a video tool-result block is dropped");
        }
    }
}
