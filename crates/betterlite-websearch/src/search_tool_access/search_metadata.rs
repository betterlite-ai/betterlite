use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

use super::hook_kwargs::{ParentOtelSpan, SearchCaller};
use super::parent_correlation::ParentRequestCorrelation;

pub type SearchRequestMetadata = IndexMap<String, SearchMetadataValue>;

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SearchMetadataValue {
    Text(String),
    Null,
    Json(Box<RawValue>),
    Span(ParentOtelSpan),
    Caller(SearchCaller),
}

pub trait SearchMetadataSource: Send + Sync {
    fn sanitized_key_metadata(&self, caller: &SearchCaller) -> SearchRequestMetadata;
    fn logged_api_key(&self, caller: &SearchCaller) -> Option<String>;
}

#[must_use]
pub fn build_search_request_metadata<Source>(
    source: &Source,
    caller: &SearchCaller,
    search_tool_name: Option<&str>,
    correlation: &ParentRequestCorrelation,
) -> SearchRequestMetadata
where
    Source: SearchMetadataSource + ?Sized,
{
    let mut metadata = source.sanitized_key_metadata(caller);
    metadata.extend(correlation.as_search_metadata());
    metadata.insert(
        "model_group".to_owned(),
        search_tool_name.map_or(SearchMetadataValue::Null, |name| {
            SearchMetadataValue::Text(name.to_owned())
        }),
    );
    metadata.insert(
        "user_api_key".to_owned(),
        source
            .logged_api_key(caller)
            .map_or(SearchMetadataValue::Null, SearchMetadataValue::Text),
    );
    metadata.insert(
        "user_api_key_auth".to_owned(),
        SearchMetadataValue::Caller(caller.clone()),
    );
    metadata
}
