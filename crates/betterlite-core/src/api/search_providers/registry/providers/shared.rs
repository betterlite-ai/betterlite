use super::super::config::SearchBody;
use super::super::error::SearchConfigError;
use serde::Serialize;
// the encoded body a wrapper hands to the pipeline, from a provider's own typed request
pub(super) fn encoded_body<T>(typed: &T) -> Result<SearchBody, SearchConfigError>
where
    T: Serialize + ?Sized,
{
    serde_json::to_value(typed)
        .map(SearchBody)
        .map_err(|source| SearchConfigError::Encode { source })
}
