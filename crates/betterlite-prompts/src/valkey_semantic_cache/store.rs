use super::ValkeySemanticCache;
use super::error::ValkeySemanticError;
use super::index;
use super::schema::{doc_mapping, scope_tag};
use crate::arguments::json::Json;
use crate::redis_semantic_cache::{RedisSemanticCache, SemanticCacheKwargs};
use crate::semantic_cache_records::extract_semantic_prompt;

pub(crate) async fn store_value(
    cache: &ValkeySemanticCache,
    key: &str,
    value: &Json,
    kwargs: &SemanticCacheKwargs,
) -> Result<(), ValkeySemanticError> {
    let messages = kwargs
        .messages
        .as_deref()
        .filter(|messages| !messages.is_empty());
    let Some(prompt) = extract_semantic_prompt(messages, kwargs.input.as_ref())? else {
        tracing::debug!("No prompt provided for semantic caching");
        return Ok(());
    };
    let response = serde_json::to_string(value).map_err(ValkeySemanticError::Encode)?;
    let vector = cache
        .embedder
        .embed(&prompt, kwargs.metadata.as_ref())
        .await?;
    let backend = index::backend(cache).await?;
    index::ensure_index(cache, backend.as_ref(), vector.len()).await?;
    let id = cache.doc_ids.next_id();
    let document = format!("{}{}:{id}", cache.key_prefix, scope_tag(key));
    let fields = doc_mapping(key, &prompt, &response, &vector);
    backend
        .store_hash(&document, &fields, RedisSemanticCache::get_ttl(kwargs.ttl))
        .await
}
