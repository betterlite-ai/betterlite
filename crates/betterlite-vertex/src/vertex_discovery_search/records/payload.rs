use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::Number;
use thiserror::Error;

use super::super::error::{ProviderError, VertexDiscoverySearchError};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::vertex_rag_store::records::payload::Reply;

mod projection;

const SEARCH_PAGE_OBJECT: &str = "vector_store.search_results.page";

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("the Vertex AI Search payload is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the Vertex AI Search payload field {0} must be {1}")]
    Field(&'static str, &'static str),
    #[error("the Vertex AI Search payload is {size} bytes, over the {limit} byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("the Vertex AI Search payload root is not a JSON object")]
    Root,
}

// LiteLLM's transform_search_vector_store_response, with the search query the call logged
pub fn transform_search_response(
    reply: &Reply<'_>,
    logging: &BTreeMap<String, Json>,
) -> Result<VectorStoreSearchResponse, VertexDiscoverySearchError> {
    let query = match logging.get("query") {
        None | Some(Json::Null) => "",
        Some(Json::Text(text)) => text,
        Some(_) => return Err(VertexDiscoverySearchError::ParamType("query", "a string")),
    };
    decode_vertex_search_payload(reply.body, query).map_err(|error| {
        VertexDiscoverySearchError::Provider(ProviderError {
            status_code: reply.status,
            message: error.to_string(),
            headers: reply
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
                .collect(),
        })
    })
}

// a bounded reply of Discovery Engine search hits, projected into the B1058 page
pub fn decode_vertex_search_payload(
    body: &[u8],
    search_query: &str,
) -> Result<VectorStoreSearchResponse, PayloadError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(PayloadError::TooLarge {
            size: body.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    if !body.trim_ascii_start().starts_with(b"{") {
        return Err(PayloadError::Root);
    }
    let hits = decode_object::<SearchPayload>(body)?
        .results
        .unwrap_or_default();
    let mut data = Vec::with_capacity(hits.len());
    for (position, hit) in hits.iter().enumerate() {
        let positional = positional_score(position)?;
        data.push(match &hit.chunk {
            Some(chunk) => projection::chunk_result(chunk, positional)?,
            None => projection::document_result(hit, positional)?,
        });
    }
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_PAGE_OBJECT.to_owned()),
        search_query: Some(search_query.to_owned()),
        data: Some(data),
        extra: BTreeMap::new(),
    })
}

fn decode_object<Payload>(body: &[u8]) -> Result<Payload, PayloadError>
where
    Payload: DeserializeOwned,
{
    let mut decoder = serde_json::Deserializer::from_slice(body);
    let payload = serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
        let path = error.path().to_string();
        PayloadError::Json(serde_json::Error::custom(format!(
            "{path}: {}",
            error.into_inner()
        )))
    })?;
    decoder.end().map_err(PayloadError::Json)?;
    Ok(payload)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SearchPayload {
    results: Option<Vec<Hit>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Hit {
    id: Option<String>,
    document: Option<Document>,
    chunk: Option<Chunk>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Document {
    struct_data: Option<IndexMap<String, Json>>,
    #[serde(rename = "derivedStructData")]
    derived: Option<DerivedStructData>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct DerivedStructData {
    title: Option<String>,
    link: Option<String>,
    display_link: Option<String>,
    formatted_url: Option<String>,
    snippets: Option<Vec<Snippet>>,
    #[serde(rename = "extractive_answers")]
    extractive_answers: Option<Vec<ExtractiveContent>>,
    #[serde(rename = "extractive_segments")]
    extractive_segments: Option<Vec<ExtractiveContent>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Snippet {
    snippet: Option<String>,
    #[serde(rename = "htmlSnippet")]
    html: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ExtractiveContent {
    content: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Chunk {
    id: Option<String>,
    name: Option<String>,
    content: Option<String>,
    document_metadata: Option<ChunkMetadata>,
    page_span: Option<IndexMap<String, Json>>,
    relevance_score: Option<Number>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ChunkMetadata {
    uri: Option<String>,
    title: Option<String>,
    struct_data: Option<IndexMap<String, Json>>,
}

fn positional_score(position: usize) -> Result<Score, PayloadError> {
    Decimal::ONE
        .checked_div(Decimal::from(position.saturating_add(1)))
        .map(Score)
        .ok_or(PayloadError::Field("score", "a representable decimal"))
}
