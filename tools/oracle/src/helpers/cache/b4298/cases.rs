use std::sync::{Arc, Mutex, PoisonError};

use betterlite_cache::cache_key::value::KeyValue;
use betterlite_cache::embedding::embedding_cache::pipeline::async_add_cache_pipeline;
use betterlite_cache::embedding::embedding_cache::{
    add_embedding_entry_to_cache, convert_to_cached_embedding, tokens,
};
use betterlite_cache::embedding::response::{EmbeddingRecord, EmbeddingResponse, EmbeddingUsage};
use betterlite_cache::envelope::facade::EnvelopeSettings;
use betterlite_cache::envelope::request::EnvelopeRequest;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_models::decisions::Extra;
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{
    self, AsCacheStore, AsNativeStore, RecordingBackend, RecordingNative, RecordingStore, failure,
    rebuild,
};
use crate::helpers::HelperResult;

#[derive(Deserialize)]
pub(crate) struct ConvertInput {
    pub(crate) response: EmbeddingRecord,
    pub(crate) model: Option<String>,
    pub(crate) prompt_tokens: Option<i64>,
    pub(crate) prompt_tokens_details: Option<Extra>,
}

#[derive(Deserialize)]
pub(crate) struct TokenInput {
    pub(crate) usage: Option<EmbeddingUsage>,
    pub(crate) data_count: usize,
    pub(crate) idx: usize,
}

#[derive(Deserialize)]
pub(crate) struct EntryInput {
    pub(crate) request: EnvelopeRequest,
    pub(crate) result: EmbeddingResponse,
    pub(crate) input: KeyValue,
    #[serde(default)]
    pub(crate) idx: usize,
    #[serde(default)]
    pub(crate) cache_type: Option<LiteLlmCacheType>,
    #[serde(default)]
    pub(crate) mode: Option<CacheMode>,
    #[serde(default)]
    pub(crate) settings: EnvelopeSettings,
}

#[derive(Deserialize)]
pub(crate) struct PipelineInput {
    pub(crate) request: EnvelopeRequest,
    pub(crate) result: EmbeddingResponse,
    pub(crate) cache_type: Option<LiteLlmCacheType>,
    pub(crate) mode: Option<CacheMode>,
    #[serde(default)]
    pub(crate) settings: EnvelopeSettings,
    #[serde(default)]
    pub(crate) dynamic: bool,
    #[serde(default)]
    pub(crate) backend_failure: bool,
    pub(crate) native: Option<NativeInput>,
}

#[derive(Deserialize)]
pub(crate) struct NativeInput {
    #[serde(default)]
    pub(crate) fail: bool,
    #[serde(default)]
    pub(crate) skip_after_first: bool,
}

pub(crate) fn convert(case: &ConvertInput) -> Value {
    let cached = convert_to_cached_embedding(
        &case.response,
        case.model.as_deref(),
        case.prompt_tokens,
        case.prompt_tokens_details.clone(),
    );
    json!({ "cached": cached })
}

fn token_response(case: &TokenInput) -> EmbeddingResponse {
    EmbeddingResponse {
        model: Some("text-embedding-3-small".to_owned()),
        data: (0..case.data_count)
            .map(|index| EmbeddingRecord {
                embedding: None,
                index: i64::try_from(index).ok(),
                object: Some("embedding".to_owned()),
            })
            .collect(),
        usage: case.usage.clone(),
    }
}

pub(crate) fn prompt_tokens(case: &TokenInput) -> Value {
    let response = token_response(case);
    json!({
        "prompt_tokens": tokens::per_item_prompt_tokens(&response, case.idx),
    })
}

pub(crate) fn prompt_tokens_details(case: &TokenInput) -> Value {
    let response = token_response(case);
    json!({
        "details": tokens::per_item_prompt_tokens_details(&response, case.idx),
    })
}

pub(crate) fn add_entry(case: &EntryInput) -> HelperResult {
    let backend = Arc::new(RecordingBackend::default());
    let cache = support::cache(
        Arc::clone(&backend),
        case.cache_type,
        case.mode,
        &case.settings,
    )?;
    let mut request = rebuild(&case.request);
    let entry =
        add_embedding_entry_to_cache(&cache, &mut request, &case.result, &case.input, case.idx)
            .map_err(failure)?;
    Ok(json!({
        "cache_key": entry.cache_key,
        "cached": serde_json::to_value(&entry.cached_data).map_err(failure)?,
        "ttl": entry.ttl,
    }))
}

pub(crate) fn pipeline(case: &PipelineInput) -> HelperResult {
    let backend = Arc::new(RecordingBackend {
        failure: case.backend_failure,
        sets: Mutex::new(Vec::new()),
    });
    let cache = support::cache(
        Arc::clone(&backend),
        case.cache_type,
        case.mode,
        &case.settings,
    )?;
    let dynamic = RecordingStore::default();
    let native = case.native.as_ref().map(|input| RecordingNative {
        failure: input.fail,
        skip_after_first: input.skip_after_first,
        called: Mutex::new(false),
        calls: Mutex::new(0),
        requests: Mutex::new(Vec::new()),
        responses: Mutex::new(Vec::new()),
    });
    let dynamic_ref = case
        .dynamic
        .then_some(AsCacheStore::as_cache_store(&dynamic));
    let native_ref = native.as_ref().map(AsNativeStore::as_native_store);
    let mut request = rebuild(&case.request);
    support::runtime()?.block_on(async_add_cache_pipeline(
        &cache,
        dynamic_ref,
        native_ref,
        &mut request,
        &case.result,
    ));
    Ok(pipeline_output(
        case,
        &cache,
        &backend,
        &dynamic,
        native.as_ref(),
    ))
}

fn pipeline_output(
    case: &PipelineInput,
    cache: &support::HelperCache,
    backend: &RecordingBackend,
    dynamic: &RecordingStore,
    native: Option<&RecordingNative>,
) -> Value {
    let sets = backend.sets();
    let writes = dynamic.writes();
    let native_called =
        native.is_some_and(|store| *store.called.lock().unwrap_or_else(PoisonError::into_inner));
    let method = if native_called {
        json!("native")
    } else if !writes.is_empty() {
        json!("dynamic")
    } else if sets.is_empty() {
        Value::Null
    } else {
        json!("default")
    };
    let entries = if native.is_some() {
        Vec::new()
    } else if case.dynamic {
        write_entries(&writes)
    } else {
        sets.iter()
            .map(|(key, cached, ttl)| json!({"key": key, "cached": cached, "ttl": ttl}))
            .collect()
    };
    let native_output = native.map(|store| {
        json!({
            "requests": store.requests().iter().map(|request| json!({"cache_key": request.cache_key})).collect::<Vec<_>>(),
            "responses": store.responses(),
        })
    });
    json!({
        "method": method,
        "entries": entries,
        "native": native_output,
        "errors": cache.hooks.logger.matching("ERROR", "exception in add_cache"),
        "debugs": cache.hooks.logger.matching("DEBUG", "skipping embedding cache write"),
    })
}

fn write_entries(writes: &[support::PipelineWrite]) -> Vec<Value> {
    writes
        .iter()
        .flat_map(|(cache_list, ttl)| {
            cache_list
                .iter()
                .map(move |(key, cached)| json!({"key": key, "cached": cached, "ttl": ttl}))
        })
        .collect()
}
