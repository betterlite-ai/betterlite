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

// Anthropic drops everything before the latest compaction block, earlier blocks of the same turn included
#[must_use]
pub fn slice_around_compaction_block(messages: &[Json]) -> Option<CompactionSlice> {
    for (message_index, message) in messages.iter().enumerate().rev() {
        let Json::Object(fields) = message else {
            continue;
        };
        let Some(Json::List(blocks)) = fields.get("content") else {
            continue;
        };
        let Some(block_index) = blocks
            .iter()
            .rposition(|block| block_has_type(block, "compaction"))
        else {
            continue;
        };
        let kept_blocks = blocks.get(block_index..)?;
        let block = kept_blocks.first()?.as_object()?.clone();
        let mut sliced = fields.clone();
        sliced.insert("content".to_owned(), Json::List(kept_blocks.to_vec()));
        let mut kept = vec![Json::Object(sliced)];
        kept.extend(
            messages
                .iter()
                .skip(message_index.saturating_add(1))
                .cloned(),
        );
        return Some(CompactionSlice {
            messages: kept,
            block,
        });
    }
    None
}

#[must_use]
pub fn strip_compaction_blocks(messages: &[Json]) -> Vec<Json> {
    let mut cleaned = Vec::with_capacity(messages.len());
    for message in messages {
        if let Json::Object(fields) = message
            && let Some(Json::List(blocks)) = fields.get("content")
        {
            let kept: Vec<Json> = blocks
                .iter()
                .filter(|block| !block_has_type(block, "compaction"))
                .cloned()
                .collect();
            if !kept.is_empty() {
                let mut copy = fields.clone();
                copy.insert("content".to_owned(), Json::List(kept));
                cleaned.push(Json::Object(copy));
            }
        } else {
            cleaned.push(message.clone());
        }
    }
    cleaned
}
