use async_trait::async_trait;
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::customer::{
    BudgetGaugeClock, CustomerGaugeContext,
};
use betterlite_logging::prometheus::budget_gauges::gauge::BudgetGauges;
use betterlite_logging::prometheus::budget_lookups::lookups::BudgetLookups;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, RemovableMetric, SeriesLimits,
};
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricLike};

use super::counts::{CountGauges, refresh_counts};
use super::customers::{CustomerRefresh, refresh_customers};
use super::keys::refresh_keys;
use super::orgs::refresh_orgs;
use super::source::BudgetPageSource;
use super::teams::refresh_teams;
use super::users::refresh_users;

mod tests;

// LiteLLM's PROMETHEUS_EMIT_BUDGET_METRICS_JOB_NAME, the lease name, not the job id
const LOCK_JOB_NAME: &str = "prometheus_emit_budget_metrics";

// PodLockManager behind a port: the refresh only needs the Redis check and the two lease calls
#[async_trait]
pub trait BudgetRefreshLock: Send + Sync {
    fn has_cache(&self) -> bool;

    async fn acquire_lock(&self, cronjob_id: &'static str) -> Option<bool>;

    async fn release_lock(&self, cronjob_id: &'static str);
}

#[async_trait]
impl<Cache: LockCache> BudgetRefreshLock for PodLockManager<Cache> {
    fn has_cache(&self) -> bool {
        self.cache.is_some()
    }

    async fn acquire_lock(&self, cronjob_id: &'static str) -> Option<bool> {
        Self::acquire_lock(self, cronjob_id, None, true).await
    }

    async fn release_lock(&self, cronjob_id: &'static str) {
        Self::release_lock(self, cronjob_id).await;
    }
}

pub struct BudgetRefresh<'refresh, Gauge>
where
    MetricLike<Gauge>: Collector + RemovableMetric,
{
    pub gauges: &'refresh BudgetGauges<MetricLike<Gauge>>,
    pub counts: CountGauges<'refresh, MetricLike<Gauge>>,
    pub source: Option<&'refresh dyn BudgetPageSource>,
    pub lookups: &'refresh dyn BudgetLookups,
    pub default_budget_id: Option<&'refresh str>,
    pub labels: &'refresh PrometheusLabelSettings,
    pub end_user_settings: EndUserCostTrackingSettings,
    pub tracker: &'refresh BoundedPrometheusSeriesTracker,
    pub limits: &'refresh SeriesLimits,
    pub clock: BudgetGaugeClock,
    pub lock: Option<&'refresh dyn BudgetRefreshLock>,
}

// ast-grep-ignore: no-long-names
pub async fn initialize_remaining_budget_metrics<Gauge>(refresh: &BudgetRefresh<'_, Gauge>)
where
    MetricLike<Gauge>: Collector + RemovableMetric + Send + Sync,
    <MetricLike<Gauge> as RemovableMetric>::Error: Send + Sync,
{
    let Some(lock) = refresh.lock else {
        run_refresh(refresh).await;
        return;
    };
    if lock.has_cache() {
        run_with_lock(refresh, lock).await;
    } else {
        run_refresh(refresh).await;
    }
}

async fn run_with_lock<Gauge>(refresh: &BudgetRefresh<'_, Gauge>, lock: &dyn BudgetRefreshLock)
where
    MetricLike<Gauge>: Collector + RemovableMetric + Send + Sync,
    <MetricLike<Gauge> as RemovableMetric>::Error: Send + Sync,
{
    if lock.acquire_lock(LOCK_JOB_NAME).await != Some(true) {
        return;
    }
    run_refresh(refresh).await;
    lock.release_lock(LOCK_JOB_NAME).await;
}

async fn run_refresh<Gauge>(refresh: &BudgetRefresh<'_, Gauge>)
where
    MetricLike<Gauge>: Collector + RemovableMetric + Send + Sync,
    <MetricLike<Gauge> as RemovableMetric>::Error: Send + Sync,
{
    tracing::debug!("Emitting key, team, user, org, customer budget metrics....");
    let labels = LabelSource::Values {
        settings: refresh.labels,
        end_user_settings: refresh.end_user_settings,
    };
    let now = refresh.clock.reset;
    refresh_teams(refresh.source, &refresh.gauges.team, labels, now).await;
    refresh_keys(refresh.source, &refresh.gauges.key, labels, now).await;
    refresh_users(refresh.source, &refresh.gauges.user, labels, now).await;
    refresh_orgs(refresh.source, &refresh.gauges.org, labels, now).await;
    let customer = CustomerRefresh {
        source: refresh.source,
        lookups: refresh.lookups,
        default_budget_id: refresh.default_budget_id,
        gauges: &refresh.gauges.customer,
        end_user_settings: refresh.end_user_settings,
        context: CustomerGaugeContext {
            source: labels,
            clock: refresh.clock,
            tracker: refresh.tracker,
            limits: refresh.limits,
        },
    };
    refresh_customers(&customer).await;
    refresh_counts(refresh.source, &refresh.counts).await;
}
