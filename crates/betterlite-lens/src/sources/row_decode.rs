use thiserror::Error;

#[derive(Debug, Error)]
#[error("the storage response is not a valid row list")]
pub struct RowsError(#[source] pub serde_json::Error);

mod tests;
