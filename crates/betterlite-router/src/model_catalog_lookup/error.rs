// The ways a catalog lookup fails where LiteLLM raises: a stale index, a non-text id, an unindexed wildcard candidate or a malformed alias entry.
#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    #[error("the catalog index points at row {index}, which is not in the model list")]
    InvalidRow { index: usize },
    #[error("the deployment at row {index} has a model_info.id that is not text")]
    InvalidId { index: usize },
    #[error("the wildcard candidate {model_id:?} is not in the catalog")]
    UnindexedCandidate { model_id: Option<String> },
    #[error("the model_group_alias entry {alias:?} has no {field} field")]
    MalformedAlias { alias: String, field: &'static str },
}
