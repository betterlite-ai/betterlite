use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreSearchResponse, VectorStoreSearchResult,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::Number;
use thiserror::Error;

use super::super::error::{ProviderError, VertexRagStoreError};

const SEARCH_PAGE_OBJECT: &str = "vector_store.search_results.page";
const UNKNOWN_DOCUMENT: &str = "Unknown Document";

pub struct Reply<'reply> {
    pub status: u16,
    pub headers: &'reply BTreeMap<String, String>,
    pub body: &'reply [u8],
}

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("the vertex RAG payload is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the vertex RAG payload field {0} must be {1}")]
    Field(&'static str, &'static str),
}

// serde reads a struct from a JSON array, so the root is checked first, and the path names the failing field
pub(super) fn parse<Payload>(body: &[u8]) -> Result<Payload, PayloadError>
where
    Payload: DeserializeOwned,
{
    if !body.trim_ascii_start().starts_with(b"{") {
        return Err(PayloadError::Json(serde_json::Error::custom(
            "the document root is not a JSON object",
        )));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let payload = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let path = error.path().to_string();
        serde_json::Error::custom(format!("{path}: {}", error.into_inner()))
    })?;
    deserializer.end()?;
    Ok(payload)
}

// LiteLLM's transform_search_vector_store_response, with the search query the call logged
pub fn transform_search_response(
    reply: &Reply<'_>,
    logging: &BTreeMap<String, Json>,
) -> Result<VectorStoreSearchResponse, VertexRagStoreError> {
    let query = match logging.get("query") {
        None | Some(Json::Null) => "",
        Some(Json::Text(text)) => text.as_str(),
        Some(_) => return Err(VertexRagStoreError::ParamType("query", "a string")),
    };
    decode_vertex_rag_payload(reply.body, query).map_err(|source| provider_error(reply, source))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Context {
    text: Option<String>,
    source_uri: Option<String>,
    source_display_name: Option<String>,
    score: Option<Number>,
    page_span: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Contexts {
    contexts: Option<Vec<Context>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RetrieveContexts {
    contexts: Option<Contexts>,
}

pub fn decode_vertex_rag_payload(
    body: &[u8],
    search_query: &str,
) -> Result<VectorStoreSearchResponse, PayloadError> {
    let contexts = parse::<RetrieveContexts>(body)?
        .contexts
        .and_then(|group| group.contexts)
        .unwrap_or_default();
    let data = contexts
        .into_iter()
        .map(search_result)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_PAGE_OBJECT.to_owned()),
        search_query: Some(search_query.to_owned()),
        data: Some(data),
        extra: BTreeMap::new(),
    })
}

// LiteLLM names the file by the source URI and then the display name, and keeps only truthy attributes
fn search_result(context: Context) -> Result<VectorStoreSearchResult, PayloadError> {
    let source = context.source_uri.unwrap_or_default();
    let display = context.source_display_name.unwrap_or_default();
    let mut attributes = IndexMap::new();
    if !source.is_empty() {
        attributes.insert("sourceUri".to_owned(), Json::Text(source.clone()));
    }
    if !display.is_empty() {
        attributes.insert("sourceDisplayName".to_owned(), Json::Text(display.clone()));
    }
    if let Some(span) = context.page_span.filter(|span| !span.is_empty()) {
        attributes.insert("pageSpan".to_owned(), Json::Object(span));
    }
    let score = match context.score {
        None => Score(Decimal::new(0, 1)),
        Some(number) => Score::try_from(number)
            .map_err(|_error| PayloadError::Field("score", "a decimal number"))?,
    };
    let file_id = if source.is_empty() {
        display.clone()
    } else {
        source
    };
    let filename = if display.is_empty() {
        UNKNOWN_DOCUMENT.to_owned()
    } else {
        display
    };
    let text = context.text.unwrap_or_default();
    let result = serde_json::json!({
        "score": score,
        "content": [{"text": text, "type": "text"}],
        "file_id": file_id,
        "filename": filename,
        "attributes": attributes,
    });
    Ok(serde_json::from_value(result)?)
}

// B1058's BaseLlmException shape: the reply's status and headers survive a decode failure
pub(super) fn provider_error(reply: &Reply<'_>, error: PayloadError) -> VertexRagStoreError {
    let headers = reply
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    VertexRagStoreError::Provider(ProviderError {
        status_code: reply.status,
        message: error.to_string(),
        headers,
    })
}
