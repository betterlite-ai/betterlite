#[derive(Debug, thiserror::Error)]
pub enum CatalogLookupError {
    #[error("candidate pattern: {0}")]
    Pattern(#[from] regex::Error),
    #[error("catalog entry {model} is not an object")]
    MalformedEntry { model: String },
}
