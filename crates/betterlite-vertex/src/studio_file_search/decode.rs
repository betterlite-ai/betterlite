use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::basic::VectorStoreStatus;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreFileCounts, VectorStoreResultContent,
    VectorStoreSearchResponse, VectorStoreSearchResult,
};
use serde::{Deserialize, Serialize, de::Error as _};

use super::ResponseHeaders;
use super::error::StudioFileSearchError;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::vertex_fine_tuning::records::timestamp::created_at;

pub struct RawResponse<'response> {
    pub status: u16,
    pub headers: &'response ResponseHeaders,
    pub body: &'response [u8],
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

// LiteLLM calls .get() on every one of these levels, so an array there is an error rather than a struct.
macro_rules! map_only {
    ($name:ident { $($field:ident: $wire:literal => $kind:ty),* $(,)? }) => {
        #[derive(Debug, Deserialize)]
        #[serde(try_from = "Json")]
        struct $name {
            $($field: $kind,)*
        }

        impl TryFrom<Json> for $name {
            type Error = serde_json::Error;

            fn try_from(value: Json) -> Result<Self, Self::Error> {
                let Json::Object(mut fields) = value else {
                    return Err(serde_json::Error::custom("the reply is not a JSON object"));
                };
                Ok(Self {
                    $($field: match fields.shift_remove($wire) {
                        Some(given) => serde_json::from_value(serde_json::to_value(given)?)?,
                        None => None,
                    },)*
                })
            }
        }
    };
}

map_only!(Reply {
    candidates: "candidates" => Option<Vec<Candidate>>,
});

map_only!(Candidate {
    grounding_metadata: "groundingMetadata" => Option<Grounding>,
});

map_only!(Grounding {
    grounding_chunks: "groundingChunks" => Option<Vec<Chunk>>,
    grounding_supports: "groundingSupports" => Option<Vec<Support>>,
});

map_only!(Chunk {
    retrieved_context: "retrievedContext" => Option<Retrieved>,
});

map_only!(Retrieved {
    text: "text" => Option<String>,
    uri: "uri" => Option<String>,
    title: "title" => Option<String>,
});

map_only!(Support {
    segment: "segment" => Option<Segment>,
    grounding_chunk_indices: "groundingChunkIndices" => Option<Vec<i64>>,
    confidence_scores: "confidenceScores" => Option<Vec<Score>>,
});

map_only!(Segment {
    text: "text" => Option<String>,
});

map_only!(Store {
    name: "name" => Option<String>,
    display_name: "displayName" => Option<String>,
    create_time: "createTime" => Option<String>,
});

#[derive(Debug, Serialize)]
struct ContextAttributes {
    uri: String,
    title: String,
}

#[derive(Debug, Serialize)]
struct SupportAttributes {
    grounding_chunk_indices: Vec<i64>,
}

pub fn decode_studio_file_search(
    shape: Shape,
    response: &RawResponse<'_>,
    query: &str,
    local: &LocalTimeContext,
) -> Result<Decoded, StudioFileSearchError> {
    match shape {
        Shape::Create => Ok(Decoded::Create(Box::new(create_response(response, local)?))),
        Shape::Search => Ok(Decoded::Search(Box::new(search_response(response, query)?))),
    }
}

fn search_response(
    response: &RawResponse<'_>,
    query: &str,
) -> Result<VectorStoreSearchResponse, StudioFileSearchError> {
    let reply: Reply = serde_json::from_slice(response.body)
        .map_err(|source| response.failure(Shape::Search, source))?;
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
            results.push(context_result(context)?);
        }
        for support in grounding.grounding_supports.unwrap_or_default() {
            if let Some(result) = support_result(support, &results)? {
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

fn context_result(context: Retrieved) -> Result<VectorStoreSearchResult, StudioFileSearchError> {
    let text = context.text.unwrap_or_default();
    let uri = context.uri.unwrap_or_default();
    let title = context.title.unwrap_or_default();
    let mut extra = BTreeMap::from([("score".to_owned(), serde_json::to_value(Json::Null)?)]);
    let file_id = (!uri.is_empty()).then_some(uri.clone());
    let filename = (!title.is_empty()).then_some(title.clone());
    if file_id.is_none() {
        extra.insert("file_id".to_owned(), serde_json::to_value(Json::Null)?);
    }
    if filename.is_none() {
        extra.insert("filename".to_owned(), serde_json::to_value(Json::Null)?);
    }
    Ok(VectorStoreSearchResult {
        score: None,
        content: Some(vec![part(&text)]),
        file_id,
        filename,
        attributes: Some(serde_json::to_value(ContextAttributes { uri, title })?),
        extra,
    })
}

fn part(text: &str) -> VectorStoreResultContent {
    VectorStoreResultContent {
        text: Some(text.to_owned()),
        type_name: Some("text".to_owned()),
        extra: BTreeMap::new(),
    }
}

fn support_result(
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
    let scores = support.confidence_scores.unwrap_or_default();
    let score = scores.into_iter().next();
    if score.is_none() {
        extra.insert("score".to_owned(), serde_json::to_value(Json::Null)?);
    }
    let attributes = SupportAttributes {
        grounding_chunk_indices: support.grounding_chunk_indices.unwrap_or_default(),
    };
    Ok(Some(VectorStoreSearchResult {
        score,
        content: Some(vec![part(&text)]),
        file_id: None,
        filename: None,
        attributes: Some(serde_json::to_value(attributes)?),
        extra,
    }))
}

fn create_response(
    response: &RawResponse<'_>,
    local: &LocalTimeContext,
) -> Result<VectorStoreCreateResponse, StudioFileSearchError> {
    let store: Store = serde_json::from_slice(response.body)
        .map_err(|source| response.failure(Shape::Create, source))?;
    let mut extra = BTreeMap::new();
    for key in ["expires_after", "expires_at", "last_active_at", "metadata"] {
        extra.insert(key.to_owned(), serde_json::to_value(Json::Null)?);
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
    pub(super) fn failure(&self, shape: Shape, source: serde_json::Error) -> StudioFileSearchError {
        match shape {
            Shape::Search => StudioFileSearchError::SearchReply {
                status: self.status,
                headers: self.headers.clone(),
                source,
            },
            Shape::Create => StudioFileSearchError::CreateReply {
                status: self.status,
                headers: self.headers.clone(),
                source,
            },
        }
    }
}
