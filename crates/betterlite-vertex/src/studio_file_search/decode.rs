use std::collections::BTreeMap;

use betterlite_openai::api::vector_store_records::records::basic::VectorStoreStatus;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreFileCounts, VectorStoreResultContent,
    VectorStoreSearchResponse, VectorStoreSearchResult,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ResponseHeaders;
use super::error::StudioFileSearchError;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::vertex_fine_tuning::records::timestamp::created_at;

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    candidates: Option<Vec<Candidate>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    grounding_metadata: Option<Grounding>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Grounding {
    grounding_chunks: Option<Vec<Chunk>>,
    grounding_supports: Option<Vec<Support>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Chunk {
    retrieved_context: Option<Retrieved>,
}

#[derive(Deserialize)]
struct Retrieved {
    text: Option<String>,
    uri: Option<String>,
    title: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Support {
    segment: Option<Segment>,
    grounding_chunk_indices: Option<Vec<Value>>,
    confidence_scores: Option<Vec<Value>>,
}

#[derive(Deserialize)]
struct Segment {
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    name: Option<String>,
    display_name: Option<String>,
    create_time: Option<String>,
}

pub fn decode_studio_file_search(
    shape: Shape,
    response: &RawResponse<'_>,
    query: &str,
    local: &LocalTimeContext,
) -> Result<Decoded, StudioFileSearchError> {
    let decoded = match shape {
        Shape::Create => Decoded::Create(Box::new(create_response(response, local)?)),
        Shape::Search => Decoded::Search(Box::new(search_response(response, query)?)),
    };
    Ok(decoded)
}

fn search_response(
    response: &RawResponse<'_>,
    query: &str,
) -> Result<VectorStoreSearchResponse, StudioFileSearchError> {
    let reply: Reply = serde_json::from_slice(response.body)
        .map_err(|error| response.failure(Shape::Search, error.to_string()))?;
    let mut results = Vec::new();
    for candidate in reply.candidates.unwrap_or_default() {
        let Some(grounding) = candidate.grounding_metadata else {
            continue;
        };
        for chunk in grounding.grounding_chunks.unwrap_or_default() {
            let Some(context) = chunk.retrieved_context else {
                continue;
            };
            if context.text.is_none() && context.uri.is_none() && context.title.is_none() {
                continue;
            }
            results.push(context_result(context));
        }
        for support in grounding.grounding_supports.unwrap_or_default() {
            if let Some(result) = support_result(response, support, &results)? {
                results.push(result);
            }
        }
    }
    Ok(VectorStoreSearchResponse {
        object: Some("vector_store.search_results.page".to_owned()),
        search_query: Some(query.to_owned()),
        data: Some(results),
        extra: BTreeMap::new(),
    })
}

fn context_result(context: Retrieved) -> VectorStoreSearchResult {
    let text = context.text.unwrap_or_default();
    let uri = context.uri.unwrap_or_default();
    let title = context.title.unwrap_or_default();
    let mut extra = BTreeMap::from([("score".to_owned(), Value::Null)]);
    let file_id = if uri.is_empty() {
        extra.insert("file_id".to_owned(), Value::Null);
        None
    } else {
        Some(uri.clone())
    };
    let filename = if title.is_empty() {
        extra.insert("filename".to_owned(), Value::Null);
        None
    } else {
        Some(title.clone())
    };
    VectorStoreSearchResult {
        score: None,
        content: Some(vec![part(&text)]),
        file_id,
        filename,
        attributes: Some(json!({ "uri": uri, "title": title })),
        extra,
    }
}

fn part(text: &str) -> VectorStoreResultContent {
    VectorStoreResultContent {
        text: Some(text.to_owned()),
        type_name: Some("text".to_owned()),
        extra: BTreeMap::new(),
    }
}

fn support_result(
    response: &RawResponse<'_>,
    support: Support,
    results: &[VectorStoreSearchResult],
) -> Result<Option<VectorStoreSearchResult>, StudioFileSearchError> {
    let text = support
        .segment
        .and_then(|segment| segment.text)
        .unwrap_or_default();
    let seen = results.iter().any(|result| {
        result
            .content
            .as_ref()
            .and_then(|parts| parts.first())
            .and_then(|part| part.text.as_deref())
            == Some(text.as_str())
    });
    if text.is_empty() || seen {
        return Ok(None);
    }
    let mut extra = BTreeMap::new();
    let score = match support.confidence_scores.unwrap_or_default().first() {
        Some(Value::Number(number)) => Some(
            Score::try_from(number.clone())
                .map_err(|error| response.failure(Shape::Search, error.to_string()))?,
        ),
        Some(Value::Null) | None => {
            extra.insert("score".to_owned(), Value::Null);
            None
        }
        Some(_) => {
            return Err(response.failure(
                Shape::Search,
                "confidenceScores[0] is not a number".to_owned(),
            ));
        }
    };
    let attributes = json!({
        "grounding_chunk_indices": support.grounding_chunk_indices.unwrap_or_default(),
    });
    Ok(Some(VectorStoreSearchResult {
        score,
        content: Some(vec![part(&text)]),
        file_id: None,
        filename: None,
        attributes: Some(attributes),
        extra,
    }))
}

fn create_response(
    response: &RawResponse<'_>,
    local: &LocalTimeContext,
) -> Result<VectorStoreCreateResponse, StudioFileSearchError> {
    let store: Store = serde_json::from_slice(response.body)
        .map_err(|error| response.failure(Shape::Create, error.to_string()))?;
    let mut extra = BTreeMap::new();
    for key in ["expires_after", "expires_at", "last_active_at", "metadata"] {
        extra.insert(key.to_owned(), Value::Null);
    }
    Ok(VectorStoreCreateResponse {
        id: Some(store.name.unwrap_or_default()),
        object: Some("vector_store".to_owned()),
        created_at: Some(created_at(store.create_time.as_deref(), local)),
        name: Some(store.display_name.unwrap_or_default()),
        bytes: Some(0),
        file_counts: Some(VectorStoreFileCounts {
            in_progress: Some(0),
            completed: Some(0),
            failed: Some(0),
            cancelled: Some(0),
            total: Some(0),
            extra: BTreeMap::new(),
        }),
        status: Some(VectorStoreStatus::Completed),
        expires_after: None,
        expires_at: None,
        last_active_at: None,
        metadata: None,
        extra,
    })
}

impl RawResponse<'_> {
    pub(super) fn failure(&self, shape: Shape, reason: String) -> StudioFileSearchError {
        let status = self.status;
        let headers = self.headers.clone();
        if shape == Shape::Search {
            StudioFileSearchError::SearchReply {
                status,
                headers,
                reason,
            }
        } else {
            StudioFileSearchError::CreateReply {
                status,
                headers,
                reason,
            }
        }
    }
}
