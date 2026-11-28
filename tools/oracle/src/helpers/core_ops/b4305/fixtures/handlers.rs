use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::DualCacheSettings;
use betterlite_cache::handler::call_types::call_type_name;
use betterlite_cache::handler::retrieve::CacheSource;
use betterlite_cache::handler::state::{CachingHandler, HandlerDependencies, OriginalFunction};
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_core::caching::hit_callbacks::job::CacheHitSuccessJob;
use betterlite_core::caching::lookup::error::TokenCountError;
use betterlite_core::caching::lookup::hit::HitLogging;
use betterlite_core::caching::lookup::services::{LookupServices, ResponseTokens};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::owned::OwnedSuccessHandler;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;

use super::cache::Silent;
use super::input::Scenario;
use super::record::{Assembler, FixedClock, Logged, SnapshotBuilder};

pub(crate) const MODEL: &str = "gpt-4o";
pub(crate) const EMBEDDING_MODEL: &str = "text-embedding-3-small";
pub(crate) const FIXED_TIME: &str = "2024-01-01T00:00:00+00:00";
const CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai", "mode": "chat"},
    "text-embedding-3-small": {"litellm_provider": "openai", "mode": "embedding"}
}"#;

pub(crate) fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "LookupError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(crate) fn caching_handler(scenario: &Scenario) -> Result<CachingHandler, Box<HelperError>> {
    let memory = LocalMemoryOps::new(1024, Arc::new(|| 1_700_000_000.0_f64)).or_config_error()?;
    Ok(CachingHandler::new(
        HandlerDependencies {
            original_function: OriginalFunction {
                name: scenario.original_function.clone(),
                parameters: Vec::new(),
            },
            start_time: 0.0,
            redis_cache: None,
            in_memory_cache: Arc::new(memory),
            dual_settings: DualCacheSettings {
                default_in_memory_ttl: None,
                default_redis_ttl: None,
                batch_cache_expiry: None,
                max_redis_batch_cache_size: 1000,
                redis_delete_chunk_size: 1000,
            },
        },
        scenario.kwargs.clone(),
    ))
}

fn logging_state(scenario: &Scenario) -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_value(serde_json::json!({
        "model": scenario.model,
        "call_type": call_type_name(scenario.call_type),
        "start_time": FIXED_TIME,
        "litellm_call_id": "call-1",
        "function_id": "function-1",
        "litellm_trace_id": "trace-1",
    }))
    .or_value_error()?;
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .or_config_error()
}

fn owned_handler(
    snapshots: &Arc<Mutex<Vec<Logged>>>,
) -> Result<Arc<OwnedSuccessHandler>, Box<HelperError>> {
    Ok(Arc::new(OwnedSuccessHandler {
        registry: Arc::new(CallbackRegistry::new(10)),
        payload_builder: Arc::new(SnapshotBuilder {
            snapshots: Arc::clone(snapshots),
        }),
        stream_assembler: Arc::new(Assembler),
        clock: Arc::new(FixedClock {
            now: DateTime::parse_from_rfc3339(FIXED_TIME).or_value_error()?,
        }),
        excluded_fields: Vec::new(),
        known_callback: Arc::new(|_name: &str| false),
        privacy: Settings::default(),
    }))
}

pub(crate) fn hit_logging(
    scenario: &Scenario,
    snapshots: &Arc<Mutex<Vec<Logged>>>,
) -> Result<HitLogging, Box<HelperError>> {
    Ok(HitLogging {
        handler: owned_handler(snapshots)?,
        state: logging_state(scenario)?,
        correlation: CorrelationContext::default(),
    })
}

pub(crate) const fn worker_settings() -> WorkerSettings {
    WorkerSettings {
        max_queue_size: 8,
        concurrency: 2,
        event_timeout: Duration::from_secs(5),
        shutdown_drain: Duration::from_secs(5),
        timeout_summary_window: Duration::from_secs(1),
    }
}

pub(crate) struct Resolution {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Resolution {
    pub(crate) fn new() -> Result<Self, Box<HelperError>> {
        let catalog =
            ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).or_config_error()?;
        let names = [MODEL.to_owned(), EMBEDDING_MODEL.to_owned()];
        Ok(Self {
            catalog,
            known: names.into_iter().collect(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        })
    }

    pub(crate) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

pub(crate) struct Tokens;

#[async_trait]
impl ResponseTokens for Tokens {
    async fn count_response_tokens(&self, _text: &str) -> Result<i64, TokenCountError> {
        Ok(0)
    }
}

pub(crate) fn scenario_services<'svc>(
    scenario: &'svc Scenario,
    cache: Option<&'svc CacheSource<'svc, Silent, Silent>>,
    context: &'svc ResolutionContext<'svc>,
    worker: &'svc LoggingWorker<CacheHitSuccessJob>,
    tokens: &'svc Tokens,
) -> Result<LookupServices<'svc, Silent, Silent>, Box<HelperError>> {
    Ok(LookupServices {
        cache,
        cache_type: scenario.cache_type,
        supported_call_types: scenario.supported_call_types.as_deref(),
        resolution: context,
        worker,
        tokens,
        perf: Arc::new(|| Duration::ZERO),
        wall: Arc::new(FixedClock {
            now: DateTime::parse_from_rfc3339(FIXED_TIME).or_value_error()?,
        }),
    })
}
