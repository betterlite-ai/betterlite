use thiserror::Error;

#[derive(Debug, Error)]
pub enum PatternCatalogError {
    #[error("the wildcard pattern {pattern:?} cannot be compiled: {source}")]
    Compile {
        pattern: String,
        #[source]
        source: regex::Error,
    },
}
