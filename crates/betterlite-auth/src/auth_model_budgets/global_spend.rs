use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::backend::{BackendError, CacheBackend, Ttl};
use betterlite_cache::coordinator::{
    AsyncCacheProtocol, CoordinatorError, EventDrivenCacheCoordinator,
};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_reset_policy::cache_invalidation::GLOBAL_PROXY_SPEND_CACHE_KEY;
use betterlite_db::database_failures::deadline::{DbLookupError, LookupRequest, bounded_db_lookup};
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use super::rows::SpendReadError;
use crate::model_budget_values::amount_text::amount_from_text;

pub const GLOBAL_SPEND_LOG_PREFIX: &str = "[GLOBAL SPEND]";
pub const PROXY_BUDGET_LOOKUP: &str = "proxy_budget";
pub const COORDINATOR_WAIT: Duration = Duration::from_secs(30);

// PostgresStore reads B723's aggregate user without creating or updating it
#[async_trait]
pub trait ProxyBudgetRows: Send + Sync {
    async fn find_proxy_budget(&self) -> Result<Option<Decimal>, SpendReadError>;
}

pub type GlobalSpendResult =
    Result<Option<Decimal>, Box<CoordinatorError<BackendError, SpendReadError>>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyBudgetAlert {
    pub admin_name: String,
    pub token: String,
    pub max_budget: Decimal,
    pub spend: Decimal,
    pub event_group: EventGroup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventGroup {
    Proxy,
}

impl ProxyBudgetAlert {
    // budget_alerts(type=...) and CallInfo.event_group on the same call
    pub const TYPE: &'static str = "proxy_budget";
}

pub trait ProxyBudgetAlertSink: Send + Sync {
    fn schedule(&self, alert: ProxyBudgetAlert);
}

pub struct GlobalSpendFetcher {
    pub rows: Arc<dyn ProxyBudgetRows>,
    pub cache: Arc<dyn CacheBackend>,
    pub coordinator: EventDrivenCacheCoordinator,
    pub tracker: Arc<DbLookupStallTracker<SystemClock>>,
    pub tasks: TaskTracker,
    pub default_deadline_seconds: f64,
    pub alerts: Arc<dyn ProxyBudgetAlertSink>,
}

// _fetch_global_spend_with_event_coordination: the coordinator runs the bounded read once and caches the null too
async fn fetch(fetcher: &GlobalSpendFetcher) -> GlobalSpendResult {
    let rows = Arc::clone(&fetcher.rows);
    let lookup = async move { Ok::<_, DbLookupError>(rows.find_proxy_budget().await) };
    let request = LookupRequest {
        name: PROXY_BUDGET_LOOKUP,
        deadline_seconds: None,
        default_deadline_seconds: fetcher.default_deadline_seconds,
        tracker: fetcher.tracker.as_ref(),
    };
    let tasks = &fetcher.tasks;
    let load = move || async move {
        let spend = bounded_db_lookup(lookup, request, tasks)
            .await
            .map_err(SpendReadError::Deadline)??;
        Ok::<_, SpendReadError>(spend.map_or_else(
            || CacheValue::Null,
            |amount| CacheValue::Text(amount.to_string()),
        ))
    };
    let cache = CachePort(fetcher.cache.as_ref());
    let cached = fetcher
        .coordinator
        .get_or_load(GLOBAL_PROXY_SPEND_CACHE_KEY, &cache, load)
        .await?;
    Ok(cached.as_ref().and_then(stored_spend))
}

// get_global_proxy_spend: no database or no positive ceiling reads nothing, and a null row schedules no alert
pub async fn get_global_proxy_spend(
    max_budget: Decimal,
    source: Option<&GlobalSpendFetcher>,
    admin_name: &str,
    token: &str,
) -> GlobalSpendResult {
    let Some(fetcher) = source else {
        return Ok(None);
    };
    if max_budget <= Decimal::ZERO {
        return Ok(None);
    }
    let spend = fetch(fetcher).await?;
    if let Some(known) = spend {
        fetcher.alerts.schedule(ProxyBudgetAlert {
            admin_name: admin_name.to_owned(),
            token: token.to_owned(),
            max_budget,
            spend: known,
            event_group: EventGroup::Proxy,
        });
    }
    Ok(spend)
}

struct CachePort<'cache>(&'cache dyn CacheBackend);

#[async_trait]
impl AsyncCacheProtocol for CachePort<'_> {
    type Error = BackendError;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, Self::Error> {
        self.0.async_get_cache(key).await
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: CacheValue,
        ttl: Ttl,
    ) -> Result<(), Self::Error> {
        self.0.async_set_cache(key, value, ttl).await
    }
}

fn stored_spend(value: &CacheValue) -> Option<Decimal> {
    match value {
        CacheValue::Text(text) => amount_from_text(text),
        CacheValue::Integer(integer) => Some(Decimal::from(*integer)),
        CacheValue::Float(float) => Decimal::from_f64_retain(*float),
        CacheValue::Credential(_)
        | CacheValue::Bytes(_)
        | CacheValue::Null
        | CacheValue::Boolean(_)
        | CacheValue::Array(_)
        | CacheValue::Object(_) => None,
    }
}
