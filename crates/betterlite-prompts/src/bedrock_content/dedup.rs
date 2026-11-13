use std::collections::HashSet;

use crate::arguments::json::{Json, Object};

use super::error::ContentError;

pub const TOOL_RESULT_BLOCK: &str = "toolResult";
pub const TOOL_USE_BLOCK: &str = "toolUse";
pub const TOOL_USE_ID: &str = "toolUseId";

pub fn deduplicate_content_blocks(
    blocks: &[Json],
    block_key: &str,
    id_key: &str,
) -> Result<Vec<Json>, ContentError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut deduplicated = Vec::with_capacity(blocks.len());
    for (index, block) in blocks.iter().enumerate() {
        let Some(fields) = block.as_object() else {
            return Err(ContentError::BlockNotObject { index });
        };
        if block_is_duplicate(fields, block_key, id_key, &mut seen)? {
            continue;
        }
        deduplicated.push(block.clone());
    }
    Ok(deduplicated)
}

fn block_is_duplicate(
    block: &Object,
    block_key: &str,
    id_key: &str,
    seen: &mut HashSet<String>,
) -> Result<bool, ContentError> {
    let Some(Json::Object(member)) = block.get(block_key) else {
        return Ok(false);
    };
    match member.get(id_key) {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Text(id)) if id.is_empty() => Ok(false),
        Some(Json::Text(id)) => {
            if seen.insert(id.clone()) {
                return Ok(false);
            }
            tracing::warn!(
                block_key,
                id_key,
                id = %id,
                "a duplicate Bedrock content block is dropped"
            );
            Ok(true)
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ContentError::BlockIdNotText {
                block_key: block_key.to_owned(),
            })
        }
    }
}
