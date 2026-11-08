use std::sync::Arc;

use super::ValkeySemanticCache;
use super::error::ValkeySemanticError;
use super::ports::{IndexSchema, ValkeyIndex};
use super::schema::assert_dim_matches;

pub(crate) async fn backend(
    cache: &ValkeySemanticCache,
) -> Result<Arc<dyn ValkeyIndex>, ValkeySemanticError> {
    if let Some(index) = &cache.index {
        return Ok(Arc::clone(index));
    }
    let url = cache
        .url
        .as_ref()
        .ok_or(ValkeySemanticError::MissingConfiguration)?;
    cache
        .live
        .get_or_try_init(|| async { cache.connector.connect(url).await })
        .await
        .map(Arc::clone)
}

pub(crate) async fn ensure_index(
    cache: &ValkeySemanticCache,
    backend: &dyn ValkeyIndex,
    dimension: usize,
) -> Result<(), ValkeySemanticError> {
    if *cache.index_dim.lock().await == Some(dimension) {
        return Ok(());
    }
    let schema = IndexSchema {
        prefix: cache.key_prefix.clone(),
        dimension,
    };
    match backend.ft_create(&cache.index_name, &schema).await {
        Ok(()) => {}
        // LiteLLM reads the lowercased exception text to tell "already exists" from another error
        Err(error) if error.to_string().to_lowercase().contains("already exists") => {
            let info = backend.ft_info(&cache.index_name).await?;
            assert_dim_matches(&info, dimension, &cache.index_name)?;
        }
        Err(error) => return Err(error),
    }
    *cache.index_dim.lock().await = Some(dimension);
    Ok(())
}
