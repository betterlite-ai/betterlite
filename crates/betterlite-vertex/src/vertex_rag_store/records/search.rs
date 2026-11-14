use std::collections::BTreeMap;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::super::error::VertexRagStoreError;
use crate::credential_sources::getters::VertexParams;
use crate::model_urls::error::VertexUrlError;

const RETRIEVE_CONTEXTS_SUFFIX: &str = ":retrieveContexts";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchQuery {
    Text(String),
    Texts(Vec<String>),
}

impl SearchQuery {
    // LiteLLM joins a query array with single spaces and leaves a text query as sent
    #[must_use]
    pub fn joined(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Texts(texts) => texts.join(" "),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCall {
    pub url: String,
    pub body: Json,
}

pub struct SearchRequest<'request> {
    pub vector_store_id: &'request str,
    pub query: &'request SearchQuery,
    pub optional_params: &'request BTreeMap<String, Json>,
    pub api_base: &'request str,
    pub logging: &'request mut BTreeMap<String, Json>,
    pub params: VertexParams<'request>,
    pub settings: &'request VertexSettings,
}

// LiteLLM's transform_search_vector_store_request: the joined query is logged for the response transform
pub fn transform_search_request(
    request: SearchRequest<'_>,
) -> Result<SearchCall, VertexRagStoreError> {
    let text = request.query.joined();
    let corpus = corpus_path(request.vector_store_id, request.params, request.settings)?;
    request
        .logging
        .insert("query".to_owned(), Json::Text(text.clone()));
    let body = search_body(text, corpus, request.optional_params)?;
    Ok(SearchCall {
        url: request.api_base.to_owned() + RETRIEVE_CONTEXTS_SUFFIX,
        body,
    })
}

fn corpus_path(
    store: &str,
    params: VertexParams<'_>,
    settings: &VertexSettings,
) -> Result<String, VertexRagStoreError> {
    if store.starts_with("projects/") {
        return Ok(store.to_owned());
    }
    let project = params
        .safe_get_vertex_ai_project(settings)
        .ok_or(VertexUrlError::ProjectRequired)?;
    let location = params
        .safe_get_vertex_ai_location(settings)
        .ok_or(VertexUrlError::LocationRequired)?;
    Ok(format!(
        "projects/{project}/locations/{location}/ragCorpora/{store}"
    ))
}

// an explicit zero max_num_results is kept, while None and null leave the key out
fn search_body(
    text: String,
    corpus: String,
    params: &BTreeMap<String, Json>,
) -> Result<Json, VertexRagStoreError> {
    let mut config = IndexMap::new();
    match params.get("max_num_results") {
        None | Some(Json::Null) => {}
        Some(Json::Number(number)) => {
            let top_k = number.as_i64().ok_or(VertexRagStoreError::ParamType(
                "max_num_results",
                "an integer",
            ))?;
            config.insert("top_k".to_owned(), Json::Number(Number::from(top_k)));
        }
        Some(_) => {
            return Err(VertexRagStoreError::ParamType(
                "max_num_results",
                "an integer",
            ));
        }
    }
    for (field, name) in [("filters", "filter"), ("ranking_options", "ranking")] {
        match params.get(field) {
            None | Some(Json::Null) => {}
            Some(Json::Object(fields)) => {
                config.insert(name.to_owned(), Json::Object(fields.clone()));
            }
            Some(_) => return Err(VertexRagStoreError::ParamType(field, "an object")),
        }
    }
    let mut query = IndexMap::from([("text".to_owned(), Json::Text(text))]);
    if !config.is_empty() {
        query.insert("rag_retrieval_config".to_owned(), Json::Object(config));
    }
    let resources = vec![Json::Object(IndexMap::from([(
        "rag_corpus".to_owned(),
        Json::Text(corpus),
    )]))];
    Ok(Json::Object(IndexMap::from([
        (
            "vertex_rag_store".to_owned(),
            Json::Object(IndexMap::from([(
                "rag_resources".to_owned(),
                Json::List(resources),
            )])),
        ),
        ("query".to_owned(), Json::Object(query)),
    ])))
}
