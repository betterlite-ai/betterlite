use betterlite_prompts::arguments::json::{Json, Object};

pub struct CompactionSlice {
    pub messages: Vec<Json>,
    pub block: Object,
}

#[must_use]
pub fn block_has_type(block: &Json, kind: &str) -> bool {
    match block {
        Json::Object(fields) => fields.get("type").and_then(Json::as_str) == Some(kind),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}

#[must_use]
pub fn listed_content(message: &Json) -> Option<(&Object, &[Json])> {
    let fields = message.as_object()?;
    match fields.get("content")? {
        Json::List(blocks) => Some((fields, blocks.as_slice())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub fn blocks_without(blocks: &[Json], kind: &str) -> Vec<Json> {
    blocks
        .iter()
        .filter(|block| !block_has_type(block, kind))
        .cloned()
        .collect()
}

#[must_use]
pub fn with_content(fields: &Object, blocks: Vec<Json>) -> Json {
    let mut copy = fields.clone();
    copy.insert("content".to_owned(), Json::List(blocks));
    Json::Object(copy)
}

// Anthropic drops everything before the latest compaction block, earlier blocks of the same turn included
#[must_use]
pub fn slice_around_compaction_block(messages: &[Json]) -> Option<CompactionSlice> {
    let (message_index, fields, kept_blocks) =
        messages
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, message)| {
                let (turn, blocks) = listed_content(message)?;
                let start = blocks
                    .iter()
                    .rposition(|block| block_has_type(block, "compaction"))?;
                Some((index, turn, blocks.get(start..)?))
            })?;
    let block = kept_blocks.first()?.as_object()?.clone();
    let mut kept = vec![with_content(fields, kept_blocks.to_vec())];
    kept.extend(
        messages
            .iter()
            .skip(message_index.saturating_add(1))
            .cloned(),
    );
    Some(CompactionSlice {
        messages: kept,
        block,
    })
}

#[must_use]
pub fn strip_compaction_blocks(messages: &[Json]) -> Vec<Json> {
    messages
        .iter()
        .filter_map(|message| {
            listed_content(message).map_or_else(
                || Some(message.clone()),
                |(fields, blocks)| {
                    let kept = blocks_without(blocks, "compaction");
                    (!kept.is_empty()).then(|| with_content(fields, kept))
                },
            )
        })
        .collect()
}
