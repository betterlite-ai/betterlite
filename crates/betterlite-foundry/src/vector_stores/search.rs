use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::embedding::{QueryEmbeddingExecutor, aembed_query, embed_query, query_text};
use super::environment::Params;
use super::error::AzureSearchError;

const SEARCH_API_VERSION: &str = "2024-07-01";
const DEFAULT_VECTOR_FIELD: &str = "contentVector";
const DEFAULT_TOP_K: i64 = 10;
const DEFAULT_SEARCH: &str = "*";
const DEFAULT_SELECT: &str = "id,content";
const VECTOR_KIND: &str = "vector";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchQuery {
    Text(String),
    Sequence(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchLogging {
    pub model_call_details: Params,
}

pub struct AzureSearchStoreRequest<'request> {
    pub vector_store_id: &'request str,
    pub query: &'request SearchQuery,
    pub optional_params: &'request Params,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub logging: &'request mut SearchLogging,
    pub embedding_executor: &'request dyn QueryEmbeddingExecutor,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchCall {
    pub url: String,
    pub body: SearchBody,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchBody {
    pub search: &'static str,
    #[serde(rename = "vectorQueries")]
    pub vector_queries: Vec<VectorQuery>,
    pub select: &'static str,
    pub top: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VectorQuery {
    pub vector: Vec<f64>,
    pub fields: String,
    pub kind: &'static str,
    #[serde(rename = "k")]
    pub top_k: i64,
}

pub fn search_request(
    request: AzureSearchStoreRequest<'_>,
) -> Result<SearchCall, AzureSearchError> {
    let text = query_text(request.query);
    let vector = embed_query(&text, request.litellm_params, request.embedding_executor)?;
    build_search_call(request, text, vector)
}

pub async fn asearch_request(
    request: AzureSearchStoreRequest<'_>,
) -> Result<SearchCall, AzureSearchError> {
    let text = query_text(request.query);
    let vector = aembed_query(&text, request.litellm_params, request.embedding_executor).await?;
    build_search_call(request, text, vector)
}

// the embedding runs before the body is built, so a bad top_k or vector field fails after the provider call
fn build_search_call(
    request: AzureSearchStoreRequest<'_>,
    query_text: String,
    vector: Vec<f64>,
) -> Result<SearchCall, AzureSearchError> {
    let vector_field = vector_field(request.litellm_params)?;
    let top_k = top_k(request.optional_params)?;
    let embedding_model = request
        .litellm_params
        .get("litellm_embedding_model")
        .cloned()
        .unwrap_or(Json::Null);
    let logging = request.logging;
    logging
        .model_call_details
        .insert("input".to_owned(), Json::Text(query_text));
    logging
        .model_call_details
        .insert("embedding_model".to_owned(), embedding_model);
    logging
        .model_call_details
        .insert("top_k".to_owned(), Json::Number(Number::from(top_k)));
    Ok(SearchCall {
        url: format!(
            "{}/indexes/{}/docs/search?api-version={SEARCH_API_VERSION}",
            request.api_base, request.vector_store_id
        ),
        body: SearchBody {
            search: DEFAULT_SEARCH,
            vector_queries: vec![VectorQuery {
                vector,
                fields: vector_field,
                kind: VECTOR_KIND,
                top_k,
            }],
            select: DEFAULT_SELECT,
            top: top_k,
        },
    })
}

fn vector_field(litellm_params: &Params) -> Result<String, AzureSearchError> {
    match litellm_params.get("azure_search_vector_field") {
        None | Some(Json::Null) => Ok(DEFAULT_VECTOR_FIELD.to_owned()),
        Some(Json::Text(field)) => Ok(field.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AzureSearchError::InvalidField {
                field: "azure_search_vector_field",
                expected: "a string",
            })
        }
    }
}

fn top_k(optional_params: &Params) -> Result<i64, AzureSearchError> {
    match optional_params.get("top_k") {
        None | Some(Json::Null) => Ok(DEFAULT_TOP_K),
        Some(Json::Number(number)) => number.as_i64().ok_or(AzureSearchError::InvalidField {
            field: "top_k",
            expected: "an integer",
        }),
        Some(Json::Text(_) | Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
            Err(AzureSearchError::InvalidField {
                field: "top_k",
                expected: "an integer",
            })
        }
    }
}
