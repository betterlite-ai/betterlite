use crate::arguments::json::{Json, Object};

use super::error::ContentError;

const CACHE_POINT: &str = "cachePoint";
const REASONING_CONTENT: &str = "reasoningContent";
const TOOL_USE: &str = "toolUse";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BlockRank {
    Reasoning,
    Content,
    Tool,
}

pub fn sort_assistant_content_blocks(blocks: &[Json]) -> Result<Vec<Json>, ContentError> {
    let mut ranked: Vec<(BlockRank, Json)> = Vec::with_capacity(blocks.len());
    for (index, block) in blocks.iter().enumerate() {
        let fields = block
            .as_object()
            .ok_or(ContentError::BlockNotObject { index })?;
        ranked.push((block_rank(fields), block.clone()));
    }
    ranked.sort_by_key(|(rank, _)| *rank);
    Ok(ranked.into_iter().map(|(_, block)| block).collect())
}

fn block_rank(block: &Object) -> BlockRank {
    if block.contains_key(REASONING_CONTENT) {
        BlockRank::Reasoning
    } else if block.contains_key(TOOL_USE) || block.contains_key(CACHE_POINT) {
        BlockRank::Tool
    } else {
        BlockRank::Content
    }
}
