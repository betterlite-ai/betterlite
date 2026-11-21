mod checks;
pub mod error;
mod facts;
mod tests;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_openai::openai_base_config::Object as JsonMap;

use self::checks::global_cap;
use self::error::LegacyAdmissionError;
use self::facts::{AdmissionFacts, CounterIds, counter_ids};
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
        let cap = global_cap(&request.metadata);
        if let Some(cap) = cap {
            checks::global(&self.handler, cap, request.model.as_deref()).await?;
        }
        let minute = self.handler.precise_minute();
        let ids = counter_ids(&facts, request.model.as_deref(), &minute);
        let mut call = Call {
            principal,
            facts,
            ids,
            model: request.model.as_deref(),
            objects: CacheObject::default(),
            counters: Vec::new(),
        };
        call.objects = self
            .handler
            .get_all_cache_objects(call.ids.read_keys(cap.is_some()), None)
            .await;
        checks::key(&self.handler, &mut call).await?;
        checks::model(&self.handler, &mut call, &mut request.metadata).await?;
        checks::user(&self.handler, &mut call).await?;
        checks::team(&self.handler, &mut call).await?;
        checks::end_user(&self.handler, &mut call).await?;
        self.write_counters(&call.counters).await;
        Ok(())
    }

    // LiteLLM detaches this write with asyncio.create_task, which the lint table here refuses; awaiting it
    // carries the same entries, ttl and local-only flag, and a failure is logged instead of failing the request
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
