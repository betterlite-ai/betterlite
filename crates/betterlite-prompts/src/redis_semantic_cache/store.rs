use super::RedisSemanticCache;
use super::SemanticCacheKwargs;
use super::error::RedisSemanticError;
use super::index;
use super::keys::entry_fields;
use crate::arguments::json::Json;
use crate::semantic_cache_records::extract_semantic_prompt;

pub(crate) async fn store_value(
    cache: &RedisSemanticCache,
    key: &str,
    value: &Json,
    kwargs: &SemanticCacheKwargs,
) -> Result<(), RedisSemanticError> {
    let messages = kwargs
        .messages
        .as_deref()
        .filter(|messages| !messages.is_empty());
    let Some(prompt) = extract_semantic_prompt(messages, kwargs.input.as_ref())? else {
        tracing::debug!("No prompt provided for semantic caching");
        return Ok(());
    };
    let response = serde_json::to_string(value).map_err(RedisSemanticError::Encode)?;
    let vector = cache
        .embedder
        .embed(&prompt, kwargs.metadata.as_ref())
        .await?;
    let index = cache
        .index
        .get_or_try_init(|| index::build_index(cache))
        .await?;
    if vector.len() != index.dimension {
        return Err(RedisSemanticError::DimensionMismatch {
            actual: vector.len(),
            expected: index.dimension,
        });
    }
    let timestamp = cache.clock.now_seconds()?;
    let fields = entry_fields(&prompt, response, &vector, key, timestamp);
    let hash_key = format!("{}:{}", index.name, fields.entry_id);
    index
        .backend
        .store_hash(&hash_key, &fields, RedisSemanticCache::get_ttl(kwargs.ttl))
        .await
}
