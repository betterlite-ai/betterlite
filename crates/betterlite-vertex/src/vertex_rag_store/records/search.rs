use std::collections::BTreeMap;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::super::error::VertexRagStoreError;
use crate::credential_sources::getters::VertexParams;
use crate::model_urls::error::VertexUrlError;

const RETRIEVE_CONTEXTS_SUFFIX: &str = ":retrieveContexts";
const DEFAULT_MAX_RESULTS_FIELD: &str = "max_num_results";
const FILTERS_FIELD: &str = "filters";
const RANKING_FIELD: &str = "ranking_options";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchBody {
    pub vertex_rag_store: RagStoreRef,
    pub query: RetrievalQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RagStoreRef {
    pub rag_resources: Vec<RagResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RagResource {
    pub rag_corpus: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetrievalQuery {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rag_retrieval_config: Option<RetrievalConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetrievalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranking: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCall {
    pub url: String,
    pub body: SearchBody,
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

// LiteLLM's transform_search_vector_store_request: the joined query is logged before the logged object backs the response
pub fn transform_search_request(
    request: SearchRequest<'_>,
) -> Result<SearchCall, VertexRagStoreError> {
    let text = request.query.joined();
    let corpus = corpus_path(request.vector_store_id, request.params, request.settings)?;
    request
        .logging
        .insert("query".to_owned(), Json::Text(text.clone()));
    let body = SearchBody {
        vertex_rag_store: RagStoreRef {
            rag_resources: vec![RagResource { rag_corpus: corpus }],
        },
        query: RetrievalQuery {
            text,
            rag_retrieval_config: retrieval_config(request.optional_params)?,
        },
    };
    Ok(SearchCall {
        url: format!("{}{RETRIEVE_CONTEXTS_SUFFIX}", request.api_base),
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

// an all-None retrieval config is left out of the body, while an explicit zero max_num_results stays in
fn retrieval_config(
    params: &BTreeMap<String, Json>,
) -> Result<Option<RetrievalConfig>, VertexRagStoreError> {
    let top_k = optional_integer(params, DEFAULT_MAX_RESULTS_FIELD)?;
    let filter = optional_object(params, FILTERS_FIELD)?;
    let ranking = optional_object(params, RANKING_FIELD)?;
    if top_k.is_none() && filter.is_none() && ranking.is_none() {
        return Ok(None);
    }
    Ok(Some(RetrievalConfig {
        top_k,
        filter,
        ranking,
    }))
}

fn optional_integer(
    params: &BTreeMap<String, Json>,
    field: &'static str,
) -> Result<Option<i64>, VertexRagStoreError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => {
            number
                .as_i64()
                .map(Some)
                .ok_or(VertexRagStoreError::ParamType {
                    field,
                    expected: "an integer",
                })
        }
        Some(_) => Err(VertexRagStoreError::ParamType {
            field,
            expected: "an integer",
        }),
    }
}

fn optional_object(
    params: &BTreeMap<String, Json>,
    field: &'static str,
) -> Result<Option<Json>, VertexRagStoreError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(value @ Json::Object(_)) => Ok(Some(value.clone())),
        Some(_) => Err(VertexRagStoreError::ParamType {
            field,
            expected: "an object",
        }),
    }
}
