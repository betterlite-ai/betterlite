use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreResultContent, VectorStoreSearchResponse, VectorStoreSearchResult,
};
use betterlite_provider::api::vector_store_contracts::config::Headers;
use betterlite_provider::api::vector_store_contracts::error::BaseLlmException;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::S3VectorsSearchError;

pub const SEARCH_RESULTS_OBJECT: &str = "vector_store.search_results.page";

pub fn decode_s3_vectors_search(
    status: u16,
    headers: &Headers,
    body: &[u8],
    query_text: Option<&str>,
) -> Result<VectorStoreSearchResponse, S3VectorsSearchError> {
    let reply = Reply { status, headers };
    let parsed =
        serde_json::from_slice::<Json>(body).map_err(|source| reply.error(source.to_string()))?;
    let Json::Object(response) = parsed else {
        return Err(reply.error("the reply is not an object".to_owned()));
    };
    let items: &[Json] = match response.get("vectors") {
        None | Some(Json::Null) => &[],
        Some(Json::List(items)) => items.as_slice(),
        Some(_) => return Err(reply.error("vectors is not an array".to_owned())),
    };
    let mut results = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if let Some(result) = search_result(&reply, index, item)? {
            results.push(result);
        }
    }
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_RESULTS_OBJECT.to_owned()),
        search_query: Some(query_text.unwrap_or_default().to_owned()),
        data: Some(results),
        extra: BTreeMap::new(),
    })
}

fn search_result(
    reply: &Reply<'_>,
    index: usize,
    item: &Json,
) -> Result<Option<VectorStoreSearchResult>, S3VectorsSearchError> {
    let Json::Object(fields) = item else {
        return Err(reply.error(format!("vectors[{index}] is not an object")));
    };
    let empty = IndexMap::new();
    let metadata = match fields.get("metadata") {
        None | Some(Json::Null) => &empty,
        Some(Json::Object(metadata)) => metadata,
        Some(_) => return Err(reply.error(format!("vectors[{index}].metadata is not an object"))),
    };
    let Some(source_text) = source_text(reply, index, metadata)? else {
        return Ok(None);
    };
    let chunk_index = chunk_index(reply, index, metadata)?;
    let filename = filename(reply, index, metadata, &chunk_index)?;
    let attributes = serde_json::to_value(Json::Object(metadata.clone()))
        .map_err(|source| reply.error(source.to_string()))?;
    Ok(Some(VectorStoreSearchResult {
        score: score(reply, index, fields.get("distance"))?,
        content: Some(vec![VectorStoreResultContent {
            text: Some(source_text.clone()),
            type_name: Some("text".to_owned()),
            extra: BTreeMap::new(),
        }]),
        file_id: Some(format!("s3-vectors-chunk-{chunk_index}")),
        filename: Some(filename),
        attributes: Some(attributes),
        extra: BTreeMap::new(),
    }))
}

fn source_text<'metadata>(
    reply: &Reply<'_>,
    index: usize,
    metadata: &'metadata IndexMap<String, Json>,
) -> Result<Option<&'metadata String>, S3VectorsSearchError> {
    match metadata.get("source_text") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) if text.is_empty() => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(reply.error(format!(
            "vectors[{index}].metadata.source_text is not a string"
        ))),
    }
}

fn filename(
    reply: &Reply<'_>,
    index: usize,
    metadata: &IndexMap<String, Json>,
    chunk_index: &str,
) -> Result<String, S3VectorsSearchError> {
    match metadata.get("filename") {
        None | Some(Json::Null) => Ok(format!("document-{chunk_index}")),
        Some(Json::Text(name)) => Ok(name.clone()),
        Some(_) => Err(reply.error(format!(
            "vectors[{index}].metadata.filename is not a string"
        ))),
    }
}

fn chunk_index(
    reply: &Reply<'_>,
    index: usize,
    metadata: &IndexMap<String, Json>,
) -> Result<String, S3VectorsSearchError> {
    match metadata.get("chunk_index") {
        None | Some(Json::Null) => Ok("0".to_owned()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Number(number)) => Ok(number.to_string()),
        Some(_) => Err(reply.error(format!(
            "vectors[{index}].metadata.chunk_index is not a string or a number"
        ))),
    }
}

#[expect(
    clippy::float_arithmetic,
    reason = "LiteLLM's score is 1.0 minus the float distance, clamped into 0 to 1"
)]
fn score(
    reply: &Reply<'_>,
    index: usize,
    distance: Option<&Json>,
) -> Result<Option<Score>, S3VectorsSearchError> {
    let invalid = |detail: &str| reply.error(format!("vectors[{index}].distance {detail}"));
    let distance = match distance {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Number(number)) => number
            .as_f64()
            .ok_or_else(|| invalid("is not a finite number"))?,
        Some(_) => return Err(invalid("is not a number")),
    };
    let clamped = (1.0_f64 - distance).clamp(0.0, 1.0);
    let number = Number::from_f64(clamped).ok_or_else(|| invalid("gives no score"))?;
    Score::try_from(number)
        .map(Some)
        .map_err(|source| invalid(&source.to_string()))
}

struct Reply<'headers> {
    status: u16,
    headers: &'headers Headers,
}

impl Reply<'_> {
    fn error(&self, message: String) -> S3VectorsSearchError {
        S3VectorsSearchError::Provider(BaseLlmException {
            status_code: self.status,
            message,
            headers: self
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
                .collect(),
        })
    }
}
