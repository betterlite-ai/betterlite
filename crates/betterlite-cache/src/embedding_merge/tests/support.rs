use betterlite_models::decisions::{Extra, HiddenParams, Json};
use chrono::{DateTime, Utc};
use serde_json::Number;

use super::fake_services::{FakeFailure, FakeServices};
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::embedding_merge::cached::{CachedLookup, CachedOutcome, process_cached_embeddings};
use crate::embedding_merge::error::ProcessError;
use crate::embedding_merge::response::{CachedEmbeddingResponse, Embedding};
use crate::embedding_merge::usage::Usage;
use crate::types::embedding::{CachedEmbedding, EmbeddingPayload};

pub(super) fn extra(text: &str) -> Extra {
    serde_json::from_str(text).expect("the fixture is a JSON object")
}

pub(super) fn largest_count() -> Extra {
    let count = Number::from_i128(i128::MAX).expect("the largest count is a number");
    Extra::from([("cached_tokens".to_owned(), Json::Number(count))])
}

pub(super) fn payload(label: &str) -> EmbeddingPayload {
    EmbeddingPayload::Text(label.to_owned())
}

pub(super) fn embedding(label: &str, index: i64) -> Embedding {
    Embedding {
        embedding: payload(label),
        index,
    }
}

pub(super) fn usage(counts: [i64; 3], details: Option<&str>) -> Usage {
    Usage {
        prompt_tokens: counts[0],
        completion_tokens: counts[1],
        total_tokens: counts[2],
        prompt_tokens_details: details.map(extra),
    }
}

pub(super) fn cached_entry(label: Option<&str>, prompt_tokens: Option<i64>) -> CachedEmbedding {
    CachedEmbedding {
        embedding: label.map(payload),
        index: Some(0),
        object: Some("embedding".to_owned()),
        model: None,
        prompt_tokens,
        prompt_tokens_details: None,
        format_version: 2,
    }
}

pub(super) fn previous_response() -> CachedEmbeddingResponse {
    CachedEmbeddingResponse {
        model: Some("previous".to_owned()),
        data: Vec::new(),
        usage: None,
        hidden_params: HiddenParams::new(),
        response_headers: None,
    }
}

pub(super) fn start() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("the timestamp is in range")
}

pub(super) fn texts(inputs: &[&str]) -> KeyValue {
    KeyValue::Array(
        inputs
            .iter()
            .map(|text| KeyValue::String((*text).to_owned()))
            .collect(),
    )
}

pub(super) fn kwargs(inputs: &[&str]) -> KeyObject {
    KeyObject::from([
        (
            "model".to_owned(),
            KeyValue::String("kwargs-model".to_owned()),
        ),
        ("input".to_owned(), texts(inputs)),
    ])
}

pub(super) async fn try_process(
    cached_result: &[Option<CachedEmbedding>],
    kwargs: &mut KeyObject,
    previous: Option<CachedEmbeddingResponse>,
    services: &FakeServices,
) -> Result<CachedOutcome, ProcessError<FakeFailure>> {
    let lookup = CachedLookup {
        cached_result,
        kwargs,
        model: "request-model",
        start_time: start(),
    };
    process_cached_embeddings(lookup, previous, services).await
}

pub(super) async fn process(
    cached_result: &[Option<CachedEmbedding>],
    kwargs: &mut KeyObject,
    previous: Option<CachedEmbeddingResponse>,
    services: &FakeServices,
) -> CachedOutcome {
    try_process(cached_result, kwargs, previous, services)
        .await
        .expect("the cached results are processed")
}
