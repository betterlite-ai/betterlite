use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::dual_cache::DualCacheSettings;
use betterlite_cache::envelope::facade::{EnvelopeSettings, ResponseCache};
use betterlite_cache::handler::call_types::{
    call_type_name, call_type_supported, should_store_result_in_cache,
};
use betterlite_cache::handler::embedding::{current_format_embedding_entry, embedding_entry_key};
use betterlite_cache::handler::retrieve::{
    AsyncSupport, CacheSource, Retrieval, retrieve_from_cache,
};
use betterlite_cache::handler::state::{CachingHandler, HandlerDependencies, OriginalFunction};
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use serde_json::json;

use super::backend::SeededBackend;
use super::input::{Eligibility, Entries, Input, Retrieve};
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

pub(crate) async fn run(input: &Input) -> HelperResult {
    match input {
        Input::Eligibility(case) => eligibility(case),
        Input::CurrentFormatEmbeddingEntry(case) => entries(case),
        Input::Retrieve(case) => retrieve(case).await,
    }
}

fn eligibility(case: &Eligibility) -> HelperResult {
    let kwargs = KeyObject::from([("cache".to_owned(), case.cache.clone())]);
    let supported = call_type_supported(&case.call_type, case.supported_call_types.as_deref());
    let store = should_store_result_in_cache(
        &case.call_type,
        case.supported_call_types.as_deref(),
        &kwargs,
    )
    .or_value_error()?;
    Ok(json!({"supported": supported, "store": store}))
}

fn entries(case: &Entries) -> HelperResult {
    let mut records = Vec::with_capacity(case.entries.len());
    for entry in &case.entries {
        match current_format_embedding_entry(entry) {
            Some(record) => records.push(serde_json::to_value(record).or_value_error()?),
            None => records.push(serde_json::Value::Null),
        }
    }
    Ok(json!(records))
}

async fn retrieve(case: &Retrieve) -> HelperResult {
    let backend = Arc::new(SeededBackend::default());
    let cache = helper_cache(Arc::clone(&backend))?;
    for entry in &case.entries {
        let key = embedding_entry_key(&cache, &case.kwargs, &entry.input).or_value_error()?;
        backend.seed(&key, envelope(&entry.record));
    }
    let mut handler = CachingHandler::new(dependencies(case)?, case.kwargs.clone());
    let source = CacheSource {
        cache: &cache,
        async_support: AsyncSupport::Supported,
    };
    let retrieval = retrieve_from_cache(
        &mut handler,
        Some(&source),
        case.call_type,
        &case.kwargs,
        &case.args,
    )
    .await
    .or_value_error()?;
    Ok(json!({
        "cached": retrieval_json(&retrieval)?,
        "preset_cache_key": handler.preset_cache_key,
        "request_kwargs": kwargs_json(&handler.request_kwargs),
    }))
}

fn dependencies(case: &Retrieve) -> Result<HandlerDependencies, Box<HelperError>> {
    Ok(HandlerDependencies {
        original_function: OriginalFunction {
            name: call_type_name(case.call_type).to_owned(),
            parameters: case.parameters.clone(),
        },
        start_time: 0.0,
        redis_cache: None,
        in_memory_cache: memory()?,
        dual_settings: dual_settings(),
    })
}

const fn dual_settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 1000,
        redis_delete_chunk_size: 1000,
    }
}

fn memory() -> Result<Arc<LocalMemoryOps>, Box<HelperError>> {
    match LocalMemoryOps::new(1024, Arc::new(|| 0.0_f64)) {
        Ok(operations) => Ok(Arc::new(operations)),
        Err(error) => Err(error).or_value_error(),
    }
}

type HelperCache = ResponseCache<SilentLogger, SilentConsole>;

struct SilentLogger;

impl LoggerSink for SilentLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        false
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, _payload: &Payload) -> bool {
        false
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

fn helper_cache(backend: Arc<SeededBackend>) -> Result<HelperCache, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    Ok(ResponseCache {
        backend,
        key_settings: CacheKeySettings {
            cache_type: LiteLlmCacheType::Local,
            mode: CacheMode::DefaultOn,
            namespace: None,
            semantic_cache_scope: SemanticCacheScope::Key,
            include_provider_params: false,
        },
        settings: EnvelopeSettings {
            ttl: None,
            redis_flush_size: None,
        },
        hooks: VerboseHooks {
            logger: SilentLogger,
            console: SilentConsole,
            set_verbose: Payload::Bool(false),
            redactor: Arc::new(redactor),
        },
        clock: Arc::new(|| 0.0_f64),
    })
}

fn envelope(record: &CacheValue) -> CacheValue {
    CacheValue::Object(BTreeMap::from([
        ("timestamp".to_owned(), CacheValue::Float(0.0)),
        ("response".to_owned(), record.clone()),
    ]))
}

fn retrieval_json(retrieval: &Retrieval) -> Result<serde_json::Value, Box<HelperError>> {
    match retrieval {
        Retrieval::Miss => Ok(serde_json::Value::Null),
        Retrieval::Hit(value) => serde_json::to_value(value).or_value_error(),
        Retrieval::Embeddings(items) => {
            let mut records = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Some(record) => records.push(serde_json::to_value(record).or_value_error()?),
                    None => records.push(serde_json::Value::Null),
                }
            }
            Ok(serde_json::Value::Array(records))
        }
    }
}

fn kwargs_json(kwargs: &KeyObject) -> serde_json::Value {
    serde_json::Value::Object(
        kwargs
            .iter()
            .map(|(name, value)| (name.clone(), key_value_json(value)))
            .collect(),
    )
}

fn key_value_json(value: &KeyValue) -> serde_json::Value {
    match value {
        KeyValue::Null => serde_json::Value::Null,
        KeyValue::Bool(flag) => serde_json::Value::Bool(*flag),
        KeyValue::Number(number) => serde_json::Value::Number(number.clone()),
        KeyValue::String(text) => serde_json::Value::String(text.clone()),
        KeyValue::Array(items) => {
            serde_json::Value::Array(items.iter().map(key_value_json).collect())
        }
        KeyValue::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), key_value_json(entry)))
                .collect(),
        ),
    }
}
