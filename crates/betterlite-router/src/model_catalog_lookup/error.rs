use crate::deployment_catalog::error::CatalogLookupError;

#[derive(Debug, thiserror::Error)]
// The ways a catalog lookup fails where LiteLLM raises: a stale index, a non-text id, the shared early and pattern resolver's refusal or a malformed alias entry.
pub enum LookupError {
    #[error("the catalog index points at row {index}, which is not in the model list")]
    InvalidRow { index: usize },
    #[error("the deployment at row {index} has a model_info.id that is not text")]
    InvalidId { index: usize },
    #[error(transparent)]
    Route(#[from] CatalogLookupError),
    #[error("the model_group_alias entry {alias:?} has no {field} field")]
    MalformedAlias { alias: String, field: &'static str },
}
