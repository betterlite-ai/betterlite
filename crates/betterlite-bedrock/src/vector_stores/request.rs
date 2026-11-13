use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::vector_stores::endpoint::retrieve_url;
use crate::vector_stores::error::VectorStoreError;
use crate::vector_stores::metadata::non_empty;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum SearchQuery {
    Text(String),
    TextList(Vec<String>),
}

impl SearchQuery {
    #[must_use]
    pub fn joined(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::TextList(items) => items.join(" "),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchOptionalParams {
    #[serde(default)]
    pub filters: Option<Json>,
    #[serde(default)]
    pub max_num_results: Option<i64>,
    #[serde(default)]
    pub ranking_options: Option<Json>,
    #[serde(default)]
    pub rewrite_query: Option<bool>,
}

pub struct SearchRequest<'request> {
    pub vector_store_id: &'request str,
    pub query: &'request SearchQuery,
    pub optional: &'request SearchOptionalParams,
    pub api_base: &'request str,
    pub litellm_params: &'request IndexMap<String, Json>,
    pub extra_body: Option<&'request IndexMap<String, Json>>,
}

pub struct SearchCall {
    pub url: String,
    pub body: IndexMap<String, Json>,
    pub query: String,
}

pub fn build_search_request(request: &SearchRequest<'_>) -> Result<SearchCall, VectorStoreError> {
    let query = request.query.joined();
    let url = retrieve_url(request.api_base, request.vector_store_id)?;
    let mut body = IndexMap::new();
    let mut retrieval_query = IndexMap::new();
    retrieval_query.insert("text".to_owned(), Json::Text(query.clone()));
    body.insert("retrievalQuery".to_owned(), Json::Object(retrieval_query));
    let mut retrieval = retrieval_configuration(request.extra_body);
    if let Some(max_results) = request.optional.max_num_results {
        apply_max_results(&mut retrieval, max_results)?;
    }
    if let Some(filters) = &request.optional.filters {
        apply_filters(&mut retrieval, filters)?;
    }
    if non_empty(&retrieval) {
        body.insert("retrievalConfiguration".to_owned(), retrieval);
    }
    if let Some(context) = user_context(request.extra_body, request.litellm_params) {
        body.insert("userContext".to_owned(), context);
    }
    Ok(SearchCall { url, body, query })
}

fn retrieval_configuration(extra_body: Option<&IndexMap<String, Json>>) -> Json {
    let Some(body) = extra_body else {
        return Json::Object(IndexMap::new());
    };
    body.get("retrievalConfiguration")
        .filter(|value| non_empty(value))
        .or_else(|| {
            body.get("retrieval_configuration")
                .filter(|value| non_empty(value))
        })
        .cloned()
        .unwrap_or_else(|| Json::Object(IndexMap::new()))
}

fn apply_max_results(retrieval: &mut Json, max_results: i64) -> Result<(), VectorStoreError> {
    let configuration = search_configuration(retrieval)?;
    let value = Json::Number(serde_json::Number::from(max_results));
    if let Some(existing) = configuration
        .get("numberOfResults")
        .filter(|prior| !matches!(prior, Json::Null))
        .filter(|prior| *prior != &value)
    {
        tracing::debug!(
            "Overriding extra_body retrievalConfiguration.vectorSearchConfiguration.numberOfResults ({}) with max_num_results={max_results}",
            dumps(existing)
        );
    }
    configuration.insert("numberOfResults".to_owned(), value);
    Ok(())
}

fn apply_filters(retrieval: &mut Json, filters: &Json) -> Result<(), VectorStoreError> {
    let configuration = search_configuration(retrieval)?;
    if configuration
        .get("filter")
        .filter(|prior| !matches!(prior, Json::Null))
        .is_some_and(|existing| existing != filters)
    {
        tracing::debug!(
            "Overriding extra_body retrievalConfiguration.vectorSearchConfiguration.filter with filters from vector_store_search_optional_params"
        );
    }
    configuration.insert("filter".to_owned(), filters.clone());
    Ok(())
}

fn search_configuration(
    retrieval: &mut Json,
) -> Result<&mut IndexMap<String, Json>, VectorStoreError> {
    let Json::Object(fields) = retrieval else {
        return Err(VectorStoreError::RetrievalConfigNotObject);
    };
    let stored = fields
        .entry("vectorSearchConfiguration".to_owned())
        .or_insert_with(|| Json::Object(IndexMap::new()));
    let Json::Object(configuration) = stored else {
        return Err(VectorStoreError::SearchConfigurationNotObject);
    };
    Ok(configuration)
}

#[must_use]
pub fn user_context(
    extra_body: Option<&IndexMap<String, Json>>,
    litellm_params: &IndexMap<String, Json>,
) -> Option<Json> {
    let sources = extra_body.into_iter().chain([litellm_params]);
    for source in sources {
        for key in ["userContext", "user_context"] {
            if let Some(value) = source.get(key).filter(|value| !matches!(value, Json::Null)) {
                return Some(value.clone());
            }
        }
    }
    None
}
