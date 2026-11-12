use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::{Extra, HiddenParams, Json};
use chrono::{DateTime, Utc};

use crate::cache_key::value::{KeyObject, KeyValue};
use crate::handler::embedding::input_items;
use crate::handler::kwargs::INPUT_KWARG;
use crate::types::embedding::CachedEmbedding;

use super::details::add_numbers;
use super::error::{MergeError, ProcessError};
use super::response::{CachedEmbeddingResponse, Embedding};
use super::usage::Usage;
use super::wrapper::build_prompt_tokens_details;

const MODEL_KWARG: &str = "model";

// What the gateway supplies: B196's response token count, and the cache-hit logging B4301 builds (provider lookup, environment update, callbacks)
#[async_trait]
pub trait CacheHitServices: Sync {
    type Error: Error + Send + 'static;

    async fn count_response_tokens(&self, text: &str) -> Result<i64, Self::Error>;

    fn log_complete_hit(&self, hit: &CompleteHit<'_>) -> Result<(), Self::Error>;
}

pub struct CompleteHit<'hit> {
    pub model: &'hit str,
    pub kwargs: &'hit KeyObject,
    pub response: &'hit CachedEmbeddingResponse,
    pub start_time: DateTime<Utc>,
}

pub struct CachedLookup<'lookup> {
    pub cached_result: &'lookup [Option<CachedEmbedding>],
    pub kwargs: &'lookup mut KeyObject,
    pub model: &'lookup str,
    pub start_time: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedOutcome {
    pub response: Option<CachedEmbeddingResponse>,
    pub all_elements_cached: bool,
}

#[derive(Default)]
struct CachedTotals {
    prompt_tokens: i64,
    details: Extra,
}

struct Split {
    remaining: Vec<KeyValue>,
    data: Vec<Option<Embedding>>,
    totals: CachedTotals,
}

// LiteLLM's _extract_model_from_cached_results: the first cached entry that names a model
#[must_use]
pub fn extract_cached_model(cached_result: &[Option<CachedEmbedding>]) -> Option<String> {
    cached_result
        .iter()
        .flatten()
        .filter_map(|entry| entry.model.as_deref())
        .find(|model| !model.is_empty())
        .map(str::to_owned)
}

// LiteLLM's _process_async_embedding_cached_response: kwargs["input"] keeps the uncached inputs, and a response is built for the cached positions
pub async fn process_cached_embeddings<Services>(
    lookup: CachedLookup<'_>,
    previous: Option<CachedEmbeddingResponse>,
    services: &Services,
) -> Result<CachedOutcome, ProcessError<Services::Error>>
where
    Services: CacheHitServices,
{
    let CachedLookup {
        cached_result,
        kwargs,
        model,
        start_time,
    } = lookup;
    let inputs = input_items(kwargs).map_err(MergeError::from)?;
    if inputs.len() != cached_result.len() {
        return Err(ProcessError::from(MergeError::CachedLength {
            cached: cached_result.len(),
            input: inputs.len(),
        }));
    }
    let kwargs_model = model_kwarg(kwargs)?;
    let split = split_cached(cached_result, &inputs, services).await?;
    let all_elements_cached = split.remaining.is_empty();
    kwargs.insert(INPUT_KWARG.to_owned(), KeyValue::Array(split.remaining));
    let response = if cached_result.iter().any(Option::is_some) {
        let model_name = extract_cached_model(cached_result).or(kwargs_model);
        Some(cached_response(model_name, split.data, split.totals))
    } else {
        previous
    };
    if all_elements_cached && let Some(built) = &response {
        let hit = CompleteHit {
            model,
            kwargs,
            response: built,
            start_time,
        };
        services
            .log_complete_hit(&hit)
            .map_err(|source| ProcessError::Service { source })?;
    }
    Ok(CachedOutcome {
        response,
        all_elements_cached,
    })
}

async fn split_cached<Services>(
    cached_result: &[Option<CachedEmbedding>],
    inputs: &[KeyValue],
    services: &Services,
) -> Result<Split, ProcessError<Services::Error>>
where
    Services: CacheHitServices,
{
    let mut split = Split {
        remaining: Vec::new(),
        data: Vec::with_capacity(inputs.len()),
        totals: CachedTotals::default(),
    };
    for (position, (slot, input)) in cached_result.iter().zip(inputs).enumerate() {
        let Some(entry) = slot else {
            split.remaining.push(input.clone());
            split.data.push(None);
            continue;
        };
        split.data.push(cached_embedding(entry, position)?);
        let counted = prompt_tokens_of(entry, input, services).await?;
        split.totals.prompt_tokens =
            split
                .totals
                .prompt_tokens
                .checked_add(counted)
                .ok_or(MergeError::Overflow {
                    field: "prompt_tokens",
                })?;
        aggregate_details(
            &mut split.totals.details,
            entry.prompt_tokens_details.as_ref(),
        )?;
    }
    Ok(split)
}

fn model_kwarg(kwargs: &KeyObject) -> Result<Option<String>, MergeError> {
    match kwargs.get(MODEL_KWARG) {
        None | Some(KeyValue::Null) => Ok(None),
        Some(KeyValue::String(model)) => Ok(Some(model.clone())),
        Some(
            KeyValue::Bool(_) | KeyValue::Number(_) | KeyValue::Array(_) | KeyValue::Object(_),
        ) => Err(MergeError::FieldType {
            field: MODEL_KWARG,
            expected: "a string",
        }),
    }
}

fn cached_embedding(
    entry: &CachedEmbedding,
    position: usize,
) -> Result<Option<Embedding>, MergeError> {
    let Some(payload) = &entry.embedding else {
        return Ok(None);
    };
    let index =
        i64::try_from(position).map_err(|_conversion| MergeError::Overflow { field: "index" })?;
    Ok(Some(Embedding {
        embedding: payload.clone(),
        index,
    }))
}

// An old entry without prompt_tokens counts its own input text, and only a string input can be counted
async fn prompt_tokens_of<Services>(
    entry: &CachedEmbedding,
    input: &KeyValue,
    services: &Services,
) -> Result<i64, ProcessError<Services::Error>>
where
    Services: CacheHitServices,
{
    if let Some(tokens) = entry.prompt_tokens {
        return Ok(tokens);
    }
    let KeyValue::String(text) = input else {
        return Ok(0);
    };
    services
        .count_response_tokens(text)
        .await
        .map_err(|source| ProcessError::Service { source })
}

// Numbers add across entries and any other value is replaced by the later entry's
fn aggregate_details(totals: &mut Extra, item: Option<&Extra>) -> Result<(), MergeError> {
    for (name, value) in item.into_iter().flatten() {
        let combined = match (totals.get(name), value) {
            (Some(Json::Number(current)), Json::Number(added)) => {
                Json::Number(add_numbers(current, added)?)
            }
            (_, other) => other.clone(),
        };
        totals.insert(name.clone(), combined);
    }
    Ok(())
}

fn cached_response(
    model: Option<String>,
    data: Vec<Option<Embedding>>,
    totals: CachedTotals,
) -> CachedEmbeddingResponse {
    let prompt_tokens_details = if totals.details.is_empty() {
        None
    } else {
        build_prompt_tokens_details(totals.details)
    };
    CachedEmbeddingResponse {
        model,
        data,
        usage: Some(Usage {
            prompt_tokens: totals.prompt_tokens,
            completion_tokens: 0,
            total_tokens: totals.prompt_tokens,
            prompt_tokens_details,
        }),
        hidden_params: HiddenParams::from([("cache_hit".to_owned(), Json::Bool(true))]),
        response_headers: None,
    }
}
