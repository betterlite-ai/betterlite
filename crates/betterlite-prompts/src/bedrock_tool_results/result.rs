use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::{Json, Object};
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
            IMAGE_URL => {
                let source = image_url_source(part, index)?;
                append_media(&mut blocks, source, remote).await?;
            }
            FILE => {
                let source = file_source(part, index)?;
                append_media(&mut blocks, source, remote).await?;
            }
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

struct MediaSource<'part> {
    image_url: &'part str,
    format: Option<&'part str>,
}

fn optional_text<'fields, WrongType>(
    fields: Option<&'fields Object>,
    key: &str,
    wrong_type: WrongType,
) -> Result<Option<&'fields str>, BedrockToolResultsError>
where
    WrongType: FnOnce() -> BedrockToolResultsError,
{
    match fields.and_then(|object| object.get(key)) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(wrong_type()),
    }
}

fn image_url_source(
    part: &ContentPart,
    index: usize,
) -> Result<MediaSource<'_>, BedrockToolResultsError> {
    match part.fields.get(IMAGE_URL) {
        None => Err(BedrockToolResultsError::ImageUrlWithoutUrl { index }),
        Some(Json::Text(image_url)) => Ok(MediaSource {
            image_url,
            format: None,
        }),
        Some(Json::Object(fields)) => {
            let image_url = optional_text(Some(fields), URL, || {
                BedrockToolResultsError::ImageUrlNotText { index }
            })?
            .ok_or(BedrockToolResultsError::ImageUrlWithoutAddress { index })?;
            let format = optional_text(Some(fields), FORMAT, || {
                BedrockToolResultsError::ImageFormatNotText { index }
            })?;
            Ok(MediaSource { image_url, format })
        }
        Some(_) => Err(BedrockToolResultsError::ImageUrlNotTextOrObject { index }),
    }
}

fn file_source(
    part: &ContentPart,
    index: usize,
) -> Result<MediaSource<'_>, BedrockToolResultsError> {
    let file = match part.fields.get(FILE) {
        None | Some(Json::Null) => None,
        Some(Json::Object(fields)) => Some(fields),
        Some(_) => return Err(BedrockToolResultsError::FileNotObject { index }),
    };
    let file_data = optional_text(file, FILE_DATA, || {
        BedrockToolResultsError::FileDataNotText { index }
    })?;
    let file_id = optional_text(file, FILE_ID, || BedrockToolResultsError::FileIdNotText {
        index,
    })?;
    let format = optional_text(file, FORMAT, || {
        BedrockToolResultsError::FileFormatNotText { index }
    })?;
    let image_url = file_id
        .or(file_data)
        .ok_or(BedrockToolResultsError::FileWithoutMedia { index })?;
    Ok(MediaSource { image_url, format })
}

async fn append_media<R>(
    blocks: &mut Vec<ToolResultPart>,
    source: MediaSource<'_>,
    remote: &RemoteImage<R>,
) -> Result<(), BedrockToolResultsError>
where
    R: Resolve,
{
    let processed = process_image(source.image_url, source.format, remote)
        .await
        .map_err(|failure| BedrockToolResultsError::Image { source: failure })?;
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
