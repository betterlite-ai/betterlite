mod checks;
pub mod error;
mod facts;
mod tests;

use std::sync::Arc;

use betterlite_auth::auth_limit_inputs::deployment::DeploymentLimitCatalog;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use tokio_util::task::TaskTracker;

use self::error::LegacyAdmissionError;
use self::facts::{AdmissionFacts, CounterIds, GLOBAL_KEY, counter_ids};
use crate::rate_limiters::legacy_rate_limit_counters::COUNTER_TTL_SECONDS;
use crate::rate_limiters::legacy_rate_limit_counters::LegacyRateLimitHandler;
use crate::rate_limiters::legacy_rate_limit_counters::objects::CacheObject;
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::LegacyCacheValue;

// the hook's `data`: the requested model and the metadata bucket earlier hooks filled
#[derive(Debug, Default)]
pub struct AdmissionRequest {
    pub model: Option<String>,
    // Object is serde_json::Map<String, Value>, which clippy.toml refuses to name by its own path
    pub metadata: JsonMap,
}

// the legacy pre-call hook, run on the handler B3047 owns
pub struct LegacyAdmission {
    // B8399 supplies the server's tracker for the detached write and B302's catalog for the deployment defaults
    pub handler: LegacyRateLimitHandler,
    pub tasks: TaskTracker,
    pub catalog: Option<Arc<dyn DeploymentLimitCatalog + Send + Sync>>,
}

// one request's state across the checks: the six counters read once, the ids they are keyed by, the collected pairs
struct Call<'call> {
    principal: &'call UserAPIKeyAuth,
    facts: AdmissionFacts,
    ids: CounterIds,
    model: Option<&'call str>,
    objects: CacheObject,
    counters: Vec<(String, LegacyCacheValue)>,
}

impl LegacyAdmission {
    pub async fn pre_call(
        &self,
        principal: &UserAPIKeyAuth,
        request: &mut AdmissionRequest,
    ) -> Result<(), LegacyAdmissionError> {
        self.handler
            .print_verbose("Inside Max Parallel Request Pre-Call Hook");
        let facts = AdmissionFacts::read(principal);
        let global_cap = request.metadata.get(GLOBAL_KEY).and_then(JsonValue::as_i64);
        if let Some(limit) = global_cap {
            checks::global(&self.handler, limit, request.model.as_deref()).await?;
        }
        let minute = self.handler.precise_minute();
        let ids = counter_ids(&facts, request.model.as_deref(), &minute);
        let objects = self
            .handler
            .get_all_cache_objects(ids.read_keys(global_cap.is_some()), None)
            .await;
        let mut call = Call {
            principal,
            facts,
            ids,
            model: request.model.as_deref(),
            objects,
            counters: Vec::new(),
        };
        if let Some(dimension) = checks::key_dimension(&call) {
            checks::enforce(&self.handler, &mut call, dimension).await?;
        }
        let catalog = self.catalog.as_deref();
        checks::model(&self.handler, &mut call, &mut request.metadata, catalog).await?;
        for dimension in checks::other_dimensions(&call) {
            checks::enforce(&self.handler, &mut call, dimension).await?;
        }
        self.write_counters(call.counters);
        Ok(())
    }

    // LiteLLM's asyncio.create_task: the write outlives the hook, and its failure is only logged
    fn write_counters(&self, entries: Vec<(String, LegacyCacheValue)>) {
        let cache = Arc::clone(&self.handler.internal_usage_cache);
        self.tasks.spawn(async move {
            let written = cache
                .batch_set(&entries, COUNTER_TTL_SECONDS, None, false)
                .await;
            if let Err(error) = written {
                tracing::error!("LiteLLM Cache: exception in async add_cache: {error}");
            }
        });
    }
}
