use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::{Json, Object};
use crate::bedrock_content::continuation::user_message_block_or_continue;
use crate::bedrock_tool_results::invoke::CachePoints;

use super::blocks::{
    CONTENT, GUARDED_TEXT, IMAGE_URL, Role, TEXT, USER, append_cache_point, block_kind,
    block_object, block_text, message_object, role_at,
};
use super::conversation::Conversion;
use super::document::document_message_block;
use super::error::ProcessorError;
use super::file::file_message_block;
use super::media::{image_block, image_source};

const FILE: &str = "file";
const GROUNDING_SOURCE: &str = "grounding_source";
const QUERY: &str = "query";
const DOCUMENT: &str = "document";

async fn user_element<R, C>(
    fields: &Object,
    kind: &str,
    block: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<Option<Json>, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    match kind {
        TEXT | GROUNDING_SOURCE | QUERY => {
            let text = block_text(fields, block)?;
            Ok(Some(Json::Object(Object::from_iter([(
                TEXT.to_owned(),
                Json::Text(text.to_owned()),
            )]))))
        }
        GUARDED_TEXT => {
            let text = block_text(fields, block)?;
            Ok(Some(Json::Object(Object::from_iter([(
                "guardContent".to_owned(),
                Json::Object(Object::from_iter([(
                    TEXT.to_owned(),
                    Json::Object(Object::from_iter([(
                        TEXT.to_owned(),
                        Json::Text(text.to_owned()),
                    )])),
                )])),
            )]))))
        }
        IMAGE_URL => {
            let (image_url, format) = image_source(fields, block)?;
            Ok(Some(
                image_block(image_url, format, block, conversion).await?,
            ))
        }
        FILE => Ok(Some(file_message_block(fields, block, conversion).await?)),
        DOCUMENT => Ok(Some(document_message_block(fields, block)?)),
        _ => Ok(None),
    }
}

async fn user_message_blocks<R, C>(
    message: &Json,
    index: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<Vec<Json>, ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    let block = user_message_block_or_continue(
        message,
        conversion.user_continue_message,
        conversion.modify_params,
    )?;
    let fields = message_object(&block, index)?;
    let mut blocks = Vec::new();
    match fields.get(CONTENT) {
        Some(Json::List(parts)) => {
            for (part_index, part) in parts.iter().enumerate() {
                let part_fields = block_object(part, part_index)?;
                let kind = block_kind(part_fields, part_index)?;
                if let Some(element) =
                    user_element(part_fields, kind, part_index, conversion).await?
                {
                    blocks.push(element);
                }
                append_cache_point(&mut blocks, part_fields, conversion);
            }
        }
        Some(Json::Text(text)) if !text.is_empty() => {
            blocks.push(Json::Object(Object::from_iter([(
                TEXT.to_owned(),
                Json::Text(text.clone()),
            )])));
            append_cache_point(&mut blocks, fields, conversion);
        }
        Some(Json::Null | Json::Text(_)) | None => {}
        Some(_) => return Err(ProcessorError::ContentNotTextOrList { message: index }),
    }
    Ok(blocks)
}

pub(crate) async fn user_turn_blocks<R, C>(
    messages: &[Json],
    start: usize,
    conversion: &Conversion<'_, C, R>,
) -> Result<(Vec<Json>, usize), ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    let mut index = start;
    let mut blocks = Vec::new();
    while let Some(message) = messages.get(index) {
        if !matches!(role_at(message, index)?, Role::Text(USER)) {
            break;
        }
        blocks.extend(user_message_blocks(message, index, conversion).await?);
        index = index.saturating_add(1);
    }
    Ok((blocks, index))
}
