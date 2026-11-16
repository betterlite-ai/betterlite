use std::sync::Arc;

use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_cache::embedding::response::EmbeddingResponse;
use betterlite_cache::envelope::facade::{Clock, EnvelopeSettings, ResponseCache};
use betterlite_cache::handler::state::OriginalFunction;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_cache::write_completion::result::CacheableResult;
use betterlite_cache::write_completion::store::{
    CacheContext, StoreRequest, WriteContext, store_response,
};
use betterlite_cache::write_completion::tracker::CacheWriteTracker;
use betterlite_cache::write_completion::wrap::{
    ResultKind,
    StreamCacheHandler,
    StreamCacheWrap,
    // ast-grep-ignore: no-long-names
    wrap_streaming_result_for_cache,
};
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::input::{Body, Input, ResultKindInput, StoreResult, StoreScenario, WrapScenario};
use super::record::{Recorder, RecordingBackend, RecordingMemory};
use crate::b4297::{HelperCache, SilentConsole, SilentLogger};

pub(crate) async fn run(input: &Input) -> HelperResult {
    let mut records = Vec::new();
    match input {
        Input::Store(input) => {
            for scenario in &input.scenarios {
                records.push(store_scenario(scenario).await?);
            }
        }
        Input::Wrap(input) => {
            for scenario in &input.scenarios {
                records.push(wrap_scenario(scenario)?);
            }
        }
    }
    Ok(Value::Array(records))
}

async fn store_scenario(scenario: &StoreScenario) -> Result<Value, Box<HelperError>> {
    let recorder = Arc::new(Recorder::default());
    let dual = if scenario.dual {
        Some(recording_dual(&recorder))
    } else {
        None
    };
    let result = cacheable_result(&scenario.result)?;
    let function = original_function(&scenario.function_name);
    let cache = if scenario.cache_configured {
        Some(response_cache(&recorder, scenario.cache_type)?)
    } else {
        None
    };
    let context = WriteContext {
        cache: CacheContext {
            cache,
            supported_call_types: scenario.supported_call_types.clone(),
        },
        tracker: Arc::new(tracker()),
    };
    store_response(
        &context,
        dual.as_ref(),
        &StoreRequest {
            result: &result,
            original_function: &function,
            kwargs: &scenario.kwargs,
            args: &scenario.args,
        },
    )
    .or_value_error()?;
    context.tracker.wait_idle().await;
    Ok(json!({ "writes": recorder.writes() }))
}

fn wrap_scenario(scenario: &WrapScenario) -> Result<Value, Box<HelperError>> {
    let function = original_function(&scenario.function_name);
    let handler = StreamCacheHandler {
        cache_configured: scenario.cache_configured,
        supported_call_types: scenario.supported_call_types.as_deref(),
        original_function: &function,
        request_kwargs: &scenario.kwargs,
    };
    // ast-grep-ignore: no-long-names
    let decision = wrap_streaming_result_for_cache(
        result_kind(scenario.result_kind),
        scenario.call_type,
        &handler,
    )
    .or_value_error()?;
    Ok(json!({ "wrapped": decision == StreamCacheWrap::Wrap }))
}

fn cacheable_result(result: &StoreResult) -> Result<CacheableResult, Box<HelperError>> {
    match result {
        StoreResult::Chat(body) => Ok(CacheableResult::Chat(body.body.clone())),
        StoreResult::Embedding(body) => Ok(CacheableResult::Embedding(embedding_response(body)?)),
        StoreResult::Raw(body) => Ok(CacheableResult::Other(body.body.clone())),
    }
}

fn embedding_response(body: &Body) -> Result<EmbeddingResponse, Box<HelperError>> {
    let value = serde_json::to_value(&body.body).or_value_error()?;
    serde_json::from_value(value).or_value_error()
}

const fn result_kind(kind: ResultKindInput) -> ResultKind {
    match kind {
        ResultKindInput::AsyncStream => ResultKind::AsyncStream,
        ResultKindInput::SyncStream => ResultKind::SyncStream,
        ResultKindInput::Response => ResultKind::Response,
    }
}

// LiteLLM takes the parameter names from the handler's callable with inspect.signature; the helper's callable takes model and messages.
fn original_function(name: &str) -> OriginalFunction {
    OriginalFunction {
        name: name.to_owned(),
        parameters: vec!["model".to_owned(), "messages".to_owned()],
    }
}

fn tracker() -> CacheWriteTracker {
    CacheWriteTracker {
        tasks: TaskTracker::new(),
        cancellation: CancellationToken::new(),
    }
}

fn recording_dual(recorder: &Arc<Recorder>) -> DualCache<RecordingMemory> {
    let memory = Arc::new(RecordingMemory::new(Arc::clone(recorder)));
    DualCache::new(memory, None, dual_settings())
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

fn response_cache(
    recorder: &Arc<Recorder>,
    cache_type: LiteLlmCacheType,
) -> Result<Arc<HelperCache>, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let clock: Clock = Arc::new(|| 0.0_f64);
    Ok(Arc::new(ResponseCache {
        backend: Arc::new(RecordingBackend::new(Arc::clone(recorder))),
        key_settings: CacheKeySettings {
            cache_type,
            mode: CacheMode::DefaultOn,
            namespace: None,
            semantic_cache_scope: SemanticCacheScope::Key,
            include_provider_params: false,
        },
        settings: EnvelopeSettings::default(),
        hooks: VerboseHooks {
            logger: SilentLogger,
            console: SilentConsole,
            set_verbose: Payload::Bool(false),
            redactor: Arc::new(redactor),
        },
        clock,
    }))
}
