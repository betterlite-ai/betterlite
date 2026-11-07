use std::error::Error as StdError;

use thiserror::Error;

use crate::arguments::json::{Json, Object};
use crate::media::inline::RemoteMediaFetcher;
use crate::media::limits::MAX_MESSAGE_PARTS;
use crate::message_content::{Content, ContentPart, Message, as_fields};

use super::document_uri::is_anthropic_document_data_uri;
use super::ids::sanitize_anthropic_tool_use_id;
use super::images::{ImageError, create_anthropic_image_param, image_format};

const TYPE: &str = "type";
const TEXT: &str = "text";
const IMAGE_URL: &str = "image_url";
const URL: &str = "url";
const TOOL_REFERENCE: &str = "tool_reference";
const TOOL_NAME: &str = "tool_name";
const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const CACHE_CONTROL: &str = "cache_control";
const TOOL_CALL_ID: &str = "tool_call_id";
const TOOL_RESULT: &str = "tool_result";
const TOOL_USE_ID: &str = "tool_use_id";
const CONTENT: &str = "content";
const TOOL_ROLE: &str = "tool";
const FUNCTION_ROLE: &str = "function";

#[derive(Debug, Error)]
pub enum ToolResultError<FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error("a tool result message must have role tool or function, not {role}")]
    Role { role: String },
    #[error("a role tool message needs a tool_call_id")]
    MissingToolCallId,
    #[error("message field {field} must be a string")]
    Field { field: &'static str },
    #[error("a content list exceeds the maximum of {limit} parts")]
    TooManyParts { limit: usize },
    #[error("the file content block could not be converted")]
    File(#[source] FileFailure),
}

// B223's anthropic_process_openai_file_message arrives through this policy, and B180 wires it to the real one
pub trait FileBlocks {
    type Failure: StdError + Send + Sync + 'static;

    fn file_block(&self, element: &Object) -> Result<Object, Self::Failure>;
}

pub async fn to_anthropic_tool_result<Source, Blocks, Ids>(
    message: &Message,
    force_base64: bool,
    fetcher: &Source,
    blocks: &Blocks,
    new_id: &Ids,
) -> Result<Object, ToolResultError<Blocks::Failure>>
where
    Source: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    let content = match message.content.as_ref() {
        None => Json::Text(String::new()),
        Some(Content::Text(text)) => Json::Text(text.clone()),
        Some(Content::Parts(parts)) => {
            Json::List(convert_parts(parts, force_base64, fetcher, blocks).await?)
        }
    };
    let tool_use_id = resolve_tool_use_id(message, new_id)?;
    let mut result = Object::from_iter([
        (TYPE.to_owned(), Json::Text(TOOL_RESULT.to_owned())),
        (TOOL_USE_ID.to_owned(), Json::Text(tool_use_id)),
        (CONTENT.to_owned(), content),
    ]);
    apply_any_cache_control(
        &mut result,
        message.provider_specific_fields.get(CACHE_CONTROL),
    );
    Ok(result)
}

fn resolve_tool_use_id<FileFailure, Ids>(
    message: &Message,
    new_id: &Ids,
) -> Result<String, ToolResultError<FileFailure>>
where
    FileFailure: StdError + Send + Sync + 'static,
    Ids: Fn() -> String,
{
    let tool_call_id = message.provider_specific_fields.get(TOOL_CALL_ID);
    match (message.role.as_str(), tool_call_id) {
        (TOOL_ROLE | FUNCTION_ROLE, Some(Json::Text(id))) => Ok(sanitize_anthropic_tool_use_id(id)),
        (TOOL_ROLE, None | Some(Json::Null)) => Err(ToolResultError::MissingToolCallId),
        (FUNCTION_ROLE, None | Some(Json::Null)) => Ok(sanitize_anthropic_tool_use_id(&new_id())),
        (TOOL_ROLE | FUNCTION_ROLE, Some(_)) => Err(ToolResultError::Field {
            field: TOOL_CALL_ID,
        }),
        (role, _) => Err(ToolResultError::Role {
            role: role.to_owned(),
        }),
    }
}

async fn convert_parts<Source, Blocks>(
    parts: &[ContentPart],
    force_base64: bool,
    fetcher: &Source,
    blocks: &Blocks,
) -> Result<Vec<Json>, ToolResultError<Blocks::Failure>>
where
    Source: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
{
    if parts.len() > MAX_MESSAGE_PARTS {
        return Err(ToolResultError::TooManyParts {
            limit: MAX_MESSAGE_PARTS,
        });
    }
    let mut converted = Vec::with_capacity(parts.len());
    for part in parts {
        if let Some(block) = convert_part(part, force_base64, fetcher, blocks).await? {
            converted.push(Json::Object(block));
        }
    }
    Ok(converted)
}

async fn convert_part<Source, Blocks>(
    part: &ContentPart,
    force_base64: bool,
    fetcher: &Source,
    blocks: &Blocks,
) -> Result<Option<Object>, ToolResultError<Blocks::Failure>>
where
    Source: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
{
    match part.kind.as_str() {
        TEXT => {
            let block = string_field_block(part, TEXT, TEXT)?;
            Ok(Some(with_any_cache_control(block, part)))
        }
        IMAGE_URL => Ok(Some(image_part(part, force_base64, fetcher, blocks).await?)),
        TOOL_REFERENCE => Ok(Some(string_field_block(part, TOOL_REFERENCE, TOOL_NAME)?)),
        FILE => Ok(Some(file_part(part, blocks)?)),
        _ => Ok(None),
    }
}

async fn image_part<Source, Blocks>(
    part: &ContentPart,
    force_base64: bool,
    fetcher: &Source,
    blocks: &Blocks,
) -> Result<Object, ToolResultError<Blocks::Failure>>
where
    Source: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
{
    let Some(image_url) = part.fields.get(IMAGE_URL) else {
        return Err(ToolResultError::Image(ImageError::Input));
    };
    let format = image_format(image_url).map_err(ToolResultError::Image)?;
    let url_str = match image_url {
        Json::Text(url) => Some(url.as_str()),
        Json::Object(fields) => fields.get(URL).and_then(Json::as_str),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    };
    if let Some(url) = url_str.filter(|url| is_anthropic_document_data_uri(url)) {
        let file = Object::from_iter([(FILE_DATA.to_owned(), Json::Text(url.to_owned()))]);
        let element = Object::from_iter([
            (TYPE.to_owned(), Json::Text(FILE.to_owned())),
            (FILE.to_owned(), Json::Object(file)),
        ]);
        let block = blocks.file_block(&element).map_err(ToolResultError::File)?;
        return Ok(with_dict_cache_control(block, part));
    }
    let block =
        create_anthropic_image_param(image_url, format.as_deref(), force_base64, fetcher).await?;
    Ok(with_dict_cache_control(block, part))
}

fn string_field_block<FileFailure>(
    part: &ContentPart,
    kind: &'static str,
    field: &'static str,
) -> Result<Object, ToolResultError<FileFailure>>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    let Some(Json::Text(value)) = part.fields.get(field) else {
        return Err(ToolResultError::Field { field });
    };
    Ok(Object::from_iter([
        (TYPE.to_owned(), Json::Text(kind.to_owned())),
        (field.to_owned(), Json::Text(value.clone())),
    ]))
}

fn file_part<Blocks>(
    part: &ContentPart,
    blocks: &Blocks,
) -> Result<Object, ToolResultError<Blocks::Failure>>
where
    Blocks: FileBlocks + Sync,
{
    let block = blocks
        .file_block(&as_fields(part))
        .map_err(ToolResultError::File)?;
    Ok(with_dict_cache_control(block, part))
}

// LiteLLM's text branch copies any non-null cache_control, while its content decorator copies only objects
fn with_any_cache_control(mut block: Object, part: &ContentPart) -> Object {
    apply_any_cache_control(&mut block, part.fields.get(CACHE_CONTROL));
    block
}

fn with_dict_cache_control(mut block: Object, part: &ContentPart) -> Object {
    let Some(Json::Object(cache_control)) = part.fields.get(CACHE_CONTROL) else {
        return block;
    };
    block.insert(
        CACHE_CONTROL.to_owned(),
        Json::Object(cache_control.clone()),
    );
    block
}

fn apply_any_cache_control(block: &mut Object, value: Option<&Json>) {
    match value {
        None | Some(Json::Null) => {}
        Some(cache_control) => {
            block.insert(CACHE_CONTROL.to_owned(), cache_control.clone());
        }
    }
}
