use thiserror::Error;

#[derive(Debug, Error)]
pub enum FusePresetsError {
    #[error("a fuse profile text is empty once its surrounding whitespace is stripped")]
    TextTooShort,
    #[error(
        "a fuse profile text is {length} characters once stripped, over the {limit} character limit"
    )]
    TextTooLong { length: usize, limit: usize },
    #[error("the bundled fuse preset catalog is not valid")]
    Catalog(#[from] serde_json::Error),
}
