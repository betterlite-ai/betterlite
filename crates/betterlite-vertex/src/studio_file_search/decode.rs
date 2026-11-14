use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreResultContent, VectorStoreSearchResponse,
    VectorStoreSearchResult,
};
use indexmap::IndexMap;
use thiserror::Error;

use super::ResponseHeaders;
use super::create::decode_create;
use super::error::StudioFileSearchError;
use crate::provider_metadata::timestamp::LocalTimeContext;

pub const SEARCH_RESULTS_OBJECT: &str = "vector_store.search_results.page";
const TEXT_KIND: &str = "text";

pub struct RawResponse<'a> {
    pub status: u16,
    pub headers: &'a ResponseHeaders,
    pub body: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Create,
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    Create(Box<VectorStoreCreateResponse>),
    Search(Box<VectorStoreSearchResponse>),
}

pub fn decode_studio_file_search(
    shape: Shape,
    response: &RawResponse<'_>,
    query: &str,
    local: &LocalTimeContext,
) -> Result<Decoded, StudioFileSearchError> {
    match shape {
        Shape::Create => {
            let record = decode_create(response, local)?;
            Ok(Decoded::Create(Box::new(record)))
        }
        Shape::Search => {
            let record = search_response(response, query)?;
            Ok(Decoded::Search(Box::new(record)))
        }
    }
}

fn search_response(
    response: &RawResponse<'_>,
    query: &str,
) -> Result<VectorStoreSearchResponse, StudioFileSearchError> {
    let parsed = serde_json::from_slice::<Json>(response.body)
        .map_err(|error| response.search_failure(error.to_string()))?;
    let Json::Object(fields) = parsed else {
        return Err(response.search_failure("the reply is not an object".to_owned()));
    };
    let candidates = reply_list(&fields, "candidates")
        .map_err(|error| response.search_failure(error.to_string()))?;
    let mut results = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        collect_candidate(response, index, candidate, &mut results)?;
    }
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_RESULTS_OBJECT.to_owned()),
        search_query: Some(query.to_owned()),
        data: Some(results),
        extra: BTreeMap::new(),
    })
}

fn collect_candidate(
    response: &RawResponse<'_>,
    index: usize,
    candidate: &Json,
    results: &mut Vec<VectorStoreSearchResult>,
) -> Result<(), StudioFileSearchError> {
    let Json::Object(candidate) = candidate else {
        return Err(response.search_failure(format!("candidates[{index}] is not an object")));
    };
    let metadata = match candidate.get("groundingMetadata") {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::Object(fields)) => fields,
        Some(_) => {
            return Err(response.search_failure(format!(
                "candidates[{index}].groundingMetadata is not an object"
            )));
        }
    };
    collect_chunks(response, index, metadata, results)?;
    collect_supports(response, index, metadata, results)
}

fn collect_chunks(
    response: &RawResponse<'_>,
    index: usize,
    metadata: &IndexMap<String, Json>,
    results: &mut Vec<VectorStoreSearchResult>,
) -> Result<(), StudioFileSearchError> {
    let chunks = reply_list(metadata, "groundingChunks")
        .map_err(|error| response.search_failure(error.to_string()))?;
    for chunk in chunks {
        let Json::Object(chunk) = chunk else {
            return Err(response.search_failure(format!(
                "candidates[{index}].groundingChunks holds a non-object"
            )));
        };
        let Some(context) = chunk.get("retrievedContext") else {
            continue;
        };
        if !betterlite_bedrock::vector_stores::metadata::non_empty(context) {
            continue;
        }
        let Json::Object(context) = context else {
            return Err(response.search_failure(format!(
                "candidates[{index}].retrievedContext is not an object"
            )));
        };
        let result = retrieved_result(response, context)?;
        results.push(result);
    }
    Ok(())
}

fn retrieved_result(
    response: &RawResponse<'_>,
    context: &IndexMap<String, Json>,
) -> Result<VectorStoreSearchResult, StudioFileSearchError> {
    let text =
        reply_text(context, "text").map_err(|error| response.search_failure(error.to_string()))?;
    let uri =
        reply_text(context, "uri").map_err(|error| response.search_failure(error.to_string()))?;
    let title =
        reply_text(context, "title").map_err(|error| response.search_failure(error.to_string()))?;
    let attributes = Json::Object(IndexMap::from([
        ("uri".to_owned(), Json::Text(uri.to_owned())),
        ("title".to_owned(), Json::Text(title.to_owned())),
    ]));
    let mut extra = BTreeMap::new();
    extra.insert("score".to_owned(), serde_json::to_value(Json::Null)?);
    let file_id = (!uri.is_empty()).then(|| uri.to_owned());
    if file_id.is_none() {
        extra.insert("file_id".to_owned(), serde_json::to_value(Json::Null)?);
    }
    let filename = (!title.is_empty()).then(|| title.to_owned());
    if filename.is_none() {
        extra.insert("filename".to_owned(), serde_json::to_value(Json::Null)?);
    }
    Ok(VectorStoreSearchResult {
        score: None,
        content: Some(vec![content(text)]),
        file_id,
        filename,
        attributes: Some(serde_json::to_value(attributes)?),
        extra,
    })
}

fn collect_supports(
    response: &RawResponse<'_>,
    index: usize,
    metadata: &IndexMap<String, Json>,
    results: &mut Vec<VectorStoreSearchResult>,
) -> Result<(), StudioFileSearchError> {
    let supports = reply_list(metadata, "groundingSupports")
        .map_err(|error| response.search_failure(error.to_string()))?;
    for support in supports {
        let Json::Object(support) = support else {
            return Err(response.search_failure(format!(
                "candidates[{index}].groundingSupports holds a non-object"
            )));
        };
        let text = support_text(response, index, support)?;
        let seen = results.iter().any(|result| {
            result
                .content
                .as_ref()
                .and_then(|content| content.first())
                .and_then(|part| part.text.as_deref())
                == Some(text)
        });
        if text.is_empty() || seen {
            continue;
        }
        let result = support_result(response, support, text)?;
        results.push(result);
    }
    Ok(())
}

fn support_text<'support>(
    response: &RawResponse<'_>,
    index: usize,
    support: &'support IndexMap<String, Json>,
) -> Result<&'support str, StudioFileSearchError> {
    let segment = match support.get("segment") {
        None | Some(Json::Null) => return Ok(""),
        Some(Json::Object(fields)) => fields,
        Some(_) => {
            return Err(response.search_failure(format!(
                "candidates[{index}].groundingSupports.segment is not an object"
            )));
        }
    };
    reply_text(segment, "text").map_err(|error| response.search_failure(error.to_string()))
}

fn support_result(
    response: &RawResponse<'_>,
    support: &IndexMap<String, Json>,
    text: &str,
) -> Result<VectorStoreSearchResult, StudioFileSearchError> {
    let indices = reply_list(support, "groundingChunkIndices")
        .map_err(|error| response.search_failure(error.to_string()))?;
    let scores = reply_list(support, "confidenceScores")
        .map_err(|error| response.search_failure(error.to_string()))?;
    let attributes = Json::Object(IndexMap::from([(
        "grounding_chunk_indices".to_owned(),
        Json::List(indices.to_vec()),
    )]));
    let mut extra = BTreeMap::new();
    let score = match scores.first() {
        None | Some(Json::Null) => {
            extra.insert("score".to_owned(), serde_json::to_value(Json::Null)?);
            None
        }
        Some(Json::Number(number)) => Some(
            Score::try_from(number.clone())
                .map_err(|error| response.search_failure(error.to_string()))?,
        ),
        Some(_) => {
            return Err(response.search_failure("confidenceScores[0] is not a number".to_owned()));
        }
    };
    Ok(VectorStoreSearchResult {
        score,
        content: Some(vec![content(text)]),
        file_id: None,
        filename: None,
        attributes: Some(serde_json::to_value(attributes)?),
        extra,
    })
}

fn content(text: &str) -> VectorStoreResultContent {
    VectorStoreResultContent {
        text: Some(text.to_owned()),
        type_name: Some(TEXT_KIND.to_owned()),
        extra: BTreeMap::new(),
    }
}

#[derive(Debug, Error)]
pub(super) enum ShapeError {
    #[error("{field} is not a string")]
    NotText { field: &'static str },
    #[error("{field} is not an array")]
    NotList { field: &'static str },
}

pub(super) fn reply_text<'a>(
    fields: &'a IndexMap<String, Json>,
    field: &'static str,
) -> Result<&'a str, ShapeError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(""),
        Some(Json::Text(text)) => Ok(text),
        Some(_) => Err(ShapeError::NotText { field }),
    }
}

pub(super) fn reply_list<'a>(
    fields: &'a IndexMap<String, Json>,
    field: &'static str,
) -> Result<&'a [Json], ShapeError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::List(items)) => Ok(items),
        Some(_) => Err(ShapeError::NotList { field }),
    }
}

impl RawResponse<'_> {
    pub(super) fn search_failure(&self, reason: String) -> StudioFileSearchError {
        StudioFileSearchError::SearchReply {
            status: self.status,
            headers: self.headers.clone(),
            reason,
        }
    }

    pub(super) fn create_failure(&self, reason: String) -> StudioFileSearchError {
        StudioFileSearchError::CreateReply {
            status: self.status,
            headers: self.headers.clone(),
            reason,
        }
    }
}
