use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheKeyError {
    #[error("metadata.caching_groups must be a list of lists of model names")]
    CachingGroups,
}
