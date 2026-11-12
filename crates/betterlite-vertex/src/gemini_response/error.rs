use std::collections::BTreeMap;

use http::header::HeaderMap;
use thiserror::Error;

use crate::gemini_candidates::error::CandidatesError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::gemini_usage::error::UsageError;
use crate::model_policy::error::VertexAiError;

#[derive(Debug, Error)]
pub enum GenerateContentError {
    #[error("the Gemini response is {size} bytes, over the {MAX_RESPONSE_BYTES} byte limit")]
    TooLarge { size: usize },
    #[error("the Gemini response is malformed at {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Usage(#[from] UsageError),
    #[error(transparent)]
    Candidates(#[from] CandidatesError),
}

impl GenerateContentError {
    #[must_use]
    pub fn into_vertex_error(self, headers: &HeaderMap) -> VertexAiError {
        VertexAiError {
            status_code: 422,
            message: self.to_string(),
            headers: Some(header_map(headers)),
        }
    }
}

pub(super) fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    let values: Vec<String> = headers
        .get_all(name)
        .iter()
        .map(|value| {
            value
                .as_bytes()
                .iter()
                .map(|byte| char::from(*byte))
                .collect()
        })
        .collect();
    (!values.is_empty()).then(|| values.join(", "))
}

fn header_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .keys()
        .filter_map(|name| {
            Some((
                name.as_str().to_owned(),
                header_text(headers, name.as_str())?,
            ))
        })
        .collect()
}
