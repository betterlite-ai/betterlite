use betterlite_http::user_url::validate::Resolve;

use crate::arguments::json::{Json, Object};
use crate::bedrock_tool_results::invoke::CachePoints;

use super::blocks::{
    GUARDED_TEXT, IMAGE_URL, TEXT, append_cache_point, block_kind, block_text, object_of,
    text_block,
};
use super::conversation::Conversion;
use super::document::document_message_block;
use super::error::ProcessorError;
use super::file::file_message_block;
use super::media::{image_block, image_source};

const DOCUMENT: &str = "document";
const FILE: &str = "file";
const GROUNDING_SOURCE: &str = "grounding_source";
const QUERY: &str = "query";

pub(crate) async fn user_list_blocks<R, C>(
    blocks: &mut Vec<Json>,
    parts: &[Json],
    conversion: &Conversion<'_, C, R>,
) -> Result<(), ProcessorError>
where
    R: Resolve,
    C: CachePoints + Sync,
{
    for (part_index, part) in parts.iter().enumerate() {
        let part_fields =
            object_of(part).ok_or(ProcessorError::BlockNotObject { block: part_index })?;
        match block_kind(part_fields, part_index)? {
            TEXT | GROUNDING_SOURCE | QUERY => {
                blocks.push(text_block(block_text(part_fields, part_index)?));
            }
            GUARDED_TEXT => {
                let text = block_text(part_fields, part_index)?;
                blocks.push(Json::Object(Object::from_iter([(
                    "guardContent".to_owned(),
                    Json::Object(Object::from_iter([(
                        TEXT.to_owned(),
                        Json::Object(Object::from_iter([(
                            TEXT.to_owned(),
                            Json::Text(text.to_owned()),
                        )])),
                    )])),
                )])));
            }
            IMAGE_URL => {
                let (image_url, format) = image_source(part_fields, part_index)?;
                blocks.push(image_block(image_url, format, part_index, conversion).await?);
            }
            FILE => blocks.push(file_message_block(part_fields, part_index, conversion).await?),
            DOCUMENT => blocks.push(document_message_block(part_fields, part_index)?),
            _ => {}
        }
        append_cache_point(blocks, part_fields, conversion);
    }
    Ok(())
}
