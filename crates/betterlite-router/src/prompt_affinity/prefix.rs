use betterlite_logging::logging_metadata::base64::{Base64Error, Base64Truncator};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::canonical::{canonical_bytes, canonical_list_bytes};
use super::json::{object, text};

// PromptCachingCache._lookback_keys: how many block positions back a pin still counts
pub const LOOKBACK_POSITIONS: i64 = 20;

const TOOL_RUN_BLOCK_TYPES: [&str; 2] = ["tool_use", "tool_result"];
const CONTENT_KEY: &str = "content";
const CACHE_CONTROL_KEY: &str = "cache_control";
const EPHEMERAL: &str = "ephemeral";

#[derive(Debug, Error)]
pub enum PrefixError {
    #[error("the base64 placeholder pass failed")]
    Truncation(#[from] Base64Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefixPosition {
    pub cache_key: String,
    pub position: i64,
}

// PromptCachingCache.prefix_positions: one digest per block of the prefix up to the last ephemeral mark
pub fn prefix_positions(
    messages: Option<&[Json]>,
    tools: Option<&[Json]>,
    truncator: &Base64Truncator,
) -> Result<Vec<PrefixPosition>, PrefixError> {
    let Some(prompt) = messages else {
        return Ok(Vec::new());
    };
    if prompt.is_empty() {
        return Ok(Vec::new());
    }
    let truncated = truncator.truncate_messages(&Json::List(cacheable_prefix(prompt)))?;
    // the truncator returns the shape it received, so the prefix is still a list of messages
    let Json::List(prefix) = truncated else {
        return Ok(Vec::new());
    };
    Ok(positions_of(&prefix, tools))
}

// PromptCachingCache.get_prompt_caching_cache_key: the key of the last cacheable block
pub fn prompt_caching_cache_key(
    messages: Option<&[Json]>,
    tools: Option<&[Json]>,
    truncator: &Base64Truncator,
) -> Result<Option<String>, PrefixError> {
    let positions = prefix_positions(messages, tools, truncator)?;
    positions
        .last()
        .map_or(Ok(None), |entry| Ok(Some(entry.cache_key.clone())))
}

// PromptCachingCache.extract_cacheable_prefix: everything up to and including the last ephemeral block
#[must_use]
pub fn cacheable_prefix(messages: &[Json]) -> Vec<Json> {
    let Some((message_index, content_index)) = last_ephemeral_mark(messages) else {
        return Vec::new();
    };
    let mut prefix: Vec<Json> = messages.iter().take(message_index).cloned().collect();
    if let Some(message) = messages.get(message_index) {
        prefix.push(prefix_message(message, content_index));
    }
    prefix
}

// PromptCachingCache._lookback_keys: the keys walked back from the last block, newest first
#[must_use]
pub fn lookback_keys(positions: &[PrefixPosition]) -> Vec<String> {
    let Some(last) = positions.last() else {
        return Vec::new();
    };
    let oldest = last.position.saturating_sub(LOOKBACK_POSITIONS);
    positions
        .iter()
        .rev()
        .filter(|entry| entry.position > oldest)
        .map(|entry| entry.cache_key.clone())
        .collect()
}

fn last_ephemeral_mark(messages: &[Json]) -> Option<(usize, Option<usize>)> {
    let mut last = None;
    for (message_index, message) in messages.iter().enumerate() {
        let Some(fields) = object(message) else {
            continue;
        };
        if is_ephemeral(fields.get(CACHE_CONTROL_KEY)) {
            last = Some((message_index, None));
        }
        let Some(Json::List(blocks)) = fields.get(CONTENT_KEY) else {
            continue;
        };
        for (block_index, block) in blocks.iter().enumerate() {
            let marked =
                object(block).is_some_and(|part| is_ephemeral(part.get(CACHE_CONTROL_KEY)));
            if marked {
                last = Some((message_index, Some(block_index)));
            }
        }
    }
    last
}

fn is_ephemeral(cache_control: Option<&Json>) -> bool {
    cache_control
        .and_then(object)
        .and_then(|fields| fields.get("type"))
        .and_then(text)
        == Some(EPHEMERAL)
}

fn prefix_message(message: &Json, content_index: Option<usize>) -> Json {
    let Some(fields) = object(message) else {
        return message.clone();
    };
    let Some(cut) = content_index else {
        return message.clone();
    };
    let Some(Json::List(blocks)) = fields.get(CONTENT_KEY) else {
        return message.clone();
    };
    let mut copy = fields.clone();
    let kept = blocks.iter().take(cut.saturating_add(1)).cloned().collect();
    copy.insert(CONTENT_KEY.to_owned(), Json::List(kept));
    Json::Object(copy)
}

fn positions_of(prefix: &[Json], tools: Option<&[Json]>) -> Vec<PrefixPosition> {
    let mut digest = seed(tools);
    let mut position: i64 = 0;
    let mut previous: Option<&'static str> = None;
    let mut positions = Vec::new();
    for message in prefix {
        for (unit, run_type) in message_units(message) {
            let continues = run_type.is_some() && run_type == previous;
            position = position.saturating_add(i64::from(!continues));
            digest = chain(&digest, &unit);
            positions.push(PrefixPosition {
                cache_key: format!("deployment:{}:prompt_caching", hex::encode(&digest)),
                position,
            });
            previous = run_type;
        }
    }
    positions
}

fn message_units(message: &Json) -> Vec<(Vec<u8>, Option<&'static str>)> {
    let Some(fields) = object(message) else {
        return Vec::new();
    };
    let envelope = sorted_pairs(fields, &[CONTENT_KEY, CACHE_CONTROL_KEY]);
    let run_type = match fields.get("role").and_then(text) {
        Some("tool") => Some("tool_result"),
        _ => None,
    };
    match fields.get(CONTENT_KEY) {
        Some(Json::List(blocks)) if !blocks.is_empty() => blocks
            .iter()
            .map(|block| block_unit(&envelope, run_type, block))
            .collect(),
        Some(Json::Text(content)) if !content.is_empty() => {
            let value = text_block(content);
            vec![(
                canonical_bytes(&Json::List(vec![envelope, value])),
                run_type,
            )]
        }
        _ => vec![(
            canonical_bytes(&Json::List(vec![envelope, Json::Null])),
            run_type,
        )],
    }
}

fn block_unit(
    envelope: &Json,
    message_run_type: Option<&'static str>,
    block: &Json,
) -> (Vec<u8>, Option<&'static str>) {
    let Some(fields) = object(block) else {
        let unmarked = canonical_bytes(&Json::List(vec![envelope.clone(), block.clone()]));
        return (unmarked, message_run_type);
    };
    let block_run_type = match fields.get("type").and_then(text) {
        Some(block_type) if TOOL_RUN_BLOCK_TYPES.contains(&block_type) => run_type(block_type),
        _ => None,
    };
    let stripped = sorted_pairs(fields, &[CACHE_CONTROL_KEY]);
    let marked = canonical_bytes(&Json::List(vec![envelope.clone(), stripped]));
    (marked, message_run_type.or(block_run_type))
}

fn run_type(block_type: &str) -> Option<&'static str> {
    match block_type {
        "tool_use" => Some("tool_use"),
        "tool_result" => Some("tool_result"),
        _ => None,
    }
}

fn text_block(content: &str) -> Json {
    pairs(&[
        ("text", Json::Text(content.to_owned())),
        ("type", Json::Text("text".to_owned())),
    ])
}

fn sorted_pairs(fields: &IndexMap<String, Json>, skipped: &[&str]) -> Json {
    let mut sorted: Vec<(&String, &Json)> = fields
        .iter()
        .filter(|(key, _)| !skipped.contains(&key.as_str()))
        .collect();
    sorted.sort_unstable_by(|left, right| left.0.cmp(right.0));
    Json::List(
        sorted
            .into_iter()
            .map(|(key, value)| Json::List(vec![Json::Text(key.to_owned()), value.clone()]))
            .collect(),
    )
}

fn pairs(entries: &[(&str, Json)]) -> Json {
    Json::List(
        entries
            .iter()
            .map(|(key, value)| Json::List(vec![Json::Text(key.to_string()), value.clone()]))
            .collect(),
    )
}

fn seed(tools: Option<&[Json]>) -> Vec<u8> {
    let bytes = tools.map_or_else(Vec::new, canonical_list_bytes);
    Sha256::digest(&bytes).to_vec()
}

fn chain(digest: &[u8], unit: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(digest);
    hasher.update(unit);
    hasher.finalize().to_vec()
}
