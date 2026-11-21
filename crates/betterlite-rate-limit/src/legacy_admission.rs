mod checks;
pub mod error;
mod facts;
mod tests;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

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
    pub handler: LegacyRateLimitHandler,
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
        checks::model(&self.handler, &mut call, &mut request.metadata).await?;
        for dimension in checks::other_dimensions(&call) {
            checks::enforce(&self.handler, &mut call, dimension).await?;
        }
        self.write_counters(&call.counters).await;
        Ok(())
    }

    // awaited instead of LiteLLM's detached asyncio.create_task: same entries, ttl and local-only flag; a failure logs
    async fn write_counters(&self, entries: &[(String, LegacyCacheValue)]) {
        let written = self
            .handler
            .internal_usage_cache
            .batch_set(entries, COUNTER_TTL_SECONDS, None, false)
            .await;
        if let Err(error) = written {
            tracing::error!("LiteLLM Cache: exception in async add_cache: {error}");
        }
    }
}
