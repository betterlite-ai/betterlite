use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::deployment_catalog::GroupAlias;
use betterlite_config::sections::rate_limiter_settings;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use indexmap::IndexMap;
use tokio::sync::Mutex as AsyncMutex;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_assembly::RateLimiterAssembly;
use crate::limiter_assembly::router_model_groups::RouterModelGroupResolver;
use crate::limiter_records::{CacheCounterValue, LimiterClock, ScriptReply};
use crate::rate_limit_window_store::{
    CounterCache, CounterScript, CounterValues, RedisCacheKind, WindowStore,
};
use crate::token_adjustments::store::{TokenAdjustments, UsageIncrements};
use crate::windowed_admission::{UuidSlotIds, WindowedAdmission};

pub(crate) struct FixedClock;

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

pub(crate) struct NoScript;

#[async_trait::async_trait]
impl CounterScript for NoScript {
    fn source(&self) -> &'static str {
        "return nil"
    }

    async fn run(
        &self,
        _keys: &[String],
        _args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        Ok(ScriptReply::Nil)
    }
}

pub(crate) struct NoCounterCache;

#[async_trait::async_trait]
impl CounterCache for NoCounterCache {
    async fn get_cache(&self, _key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        None
    }

    async fn set_cache(&self, _key: &str, _value: CacheCounterValue, _ttl: i64, _local_only: bool) {
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

pub(crate) struct NoGaugeCache;

#[async_trait::async_trait]
impl GaugeCache for NoGaugeCache {
    async fn get(&self, _key: &str, _local_only: bool) -> Option<GaugeCacheValue> {
        None
    }

    async fn set(&self, _key: &str, _value: GaugeCacheValue, _ttl: Duration, _local_only: bool) {}

    async fn batch_get(
        &self,
        _keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        None
    }
}

pub(crate) struct NoUsage;

#[async_trait::async_trait]
impl UsageIncrements for NoUsage {
    async fn increment(&self, _operation: &IncrementOperation) {}

    async fn increment_pipeline(&self, _operations: &[IncrementOperation]) {}

    async fn set_local_counter(&self, _key: &str, _value: f64, _ttl: Option<i64>) {}
}

pub(crate) struct Owners {
    pub(crate) settings: rate_limiter_settings::Settings,
    pub(crate) clock: Arc<dyn LimiterClock>,
    pub(crate) fail_closed: FailClosedPolicy,
    pub(crate) windows: Arc<WindowStore>,
    pub(crate) script: Arc<dyn CounterScript>,
    pub(crate) gauges: Arc<ConcurrencyGauges>,
    pub(crate) admission: Arc<WindowedAdmission>,
    pub(crate) adjustments: Arc<TokenAdjustments>,
    pub(crate) aliases: Arc<IndexMap<String, GroupAlias>>,
}

pub(crate) fn window_store() -> WindowStore {
    WindowStore {
        cache: Arc::new(NoCounterCache),
        rate_limiter_script: None,
        counter_read_script: None,
        request_batch: None,
        check_and_increment_lock: AsyncMutex::new(()),
    }
}

pub(crate) fn build_owners(settings: rate_limiter_settings::Settings) -> Owners {
    let fail_closed = FailClosedPolicy {
        enabled: settings.fail_closed,
    };
    let clock: Arc<dyn LimiterClock> = Arc::new(FixedClock);
    let script: Arc<dyn CounterScript> = Arc::new(NoScript);
    let windows = Arc::new(window_store());
    let gauges = build_gauges(&clock, fail_closed);
    let admission = Arc::new(WindowedAdmission {
        window_size: settings.window_size,
        clock: Arc::clone(&clock),
        fail_closed,
        windows: Arc::clone(&windows),
        refund_script: Some(Arc::clone(&script)),
        gauges: Arc::clone(&gauges),
        slot_ids: Arc::new(UuidSlotIds),
    });
    let adjustments = build_adjustments();
    let aliases = build_aliases();
    Owners {
        settings,
        clock,
        fail_closed,
        windows,
        script,
        gauges,
        admission,
        adjustments,
        aliases,
    }
}

fn build_gauges(
    clock: &Arc<dyn LimiterClock>,
    fail_closed: FailClosedPolicy,
) -> Arc<ConcurrencyGauges> {
    Arc::new(ConcurrencyGauges {
        cache: Arc::new(NoGaugeCache),
        clock: Arc::clone(clock),
        acquire_script: None,
        release_script: None,
        count_script: None,
        release_batch: None,
        fail_closed,
        check_and_increment_lock: AsyncMutex::new(()),
    })
}

fn build_adjustments() -> Arc<TokenAdjustments> {
    Arc::new(TokenAdjustments {
        windows: window_store(),
        usage: Arc::new(NoUsage),
        post_call_batch: None,
        token_increment_script: None,
        guarded_token_script: None,
    })
}

fn build_aliases() -> Arc<IndexMap<String, GroupAlias>> {
    Arc::new(IndexMap::from([(
        String::from("alias"),
        GroupAlias {
            model: String::from("group"),
            hidden: false,
        },
    )]))
}

pub(crate) fn build_assembly(owners: &Owners) -> RateLimiterAssembly {
    RateLimiterAssembly {
        settings: owners.settings,
        clock: Arc::clone(&owners.clock),
        fail_closed: owners.fail_closed,
        windows: Arc::clone(&owners.windows),
        check_and_increment_script: Some(Arc::clone(&owners.script)),
        gauges: Arc::clone(&owners.gauges),
        admission: Arc::clone(&owners.admission),
        adjustments: Arc::clone(&owners.adjustments),
        model_groups: Box::new(RouterModelGroupResolver {
            aliases: Arc::clone(&owners.aliases),
        }),
        batch_limiter: Mutex::new(None),
    }
}
