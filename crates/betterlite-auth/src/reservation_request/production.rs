use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_redis::redis_cache::RedisCache;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::budget_counter::BudgetCounter;
use crate::reservation_admission::counters::AdmissionCounters;
use crate::reservation_admission::engine::{Admission, AdmissionRequest, admit};
use crate::reservation_admission::error::AdmissionError;
use crate::reservation_cost::error::ReservationCostError;
use crate::reservation_cost::input_cost::estimate_request_input_cost;
use crate::reservation_cost::{
    EstimateRequest, EstimateSources, count_request_input_tokens, estimate_request_max_cost,
};
use crate::reservation_counters::error::ReservationCounterError;
use crate::reservation_counters::{CounterInput, CounterServices, budget_counters};
use crate::reservation_settlement::counters::{ScopeHold, SettlementCounters};
use crate::reservation_settlement::entry::ReservationEntry;
use crate::reservation_settlement::release::release_entries_best_effort;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_writes::SpendCounterWrites;
use crate::spend_writes::layers::CounterLayers;
use crate::spend_writes::ttl::refresh_spend_counter_ttl;

use super::lease::LeaseRenewal;
use super::ports::{
    ReservationAdmission, ReservationCostSource, ReservationCounterSource, ReservationLeases,
    ReservationSettlement,
};

// B3002's budget_counters over the server's cache and row handles
pub struct RequestCounters<'services> {
    pub services: CounterServices<'services>,
}

#[async_trait]
impl ReservationCounterSource for RequestCounters<'_> {
    async fn counters(
        &self,
        input: &CounterInput<'_>,
    ) -> Result<Vec<BudgetCounter>, ReservationCounterError> {
        budget_counters(input, &self.services).await
    }
}

// B3001's three estimates over the request's model metadata and tokenizer
pub struct RequestCosts<'services> {
    pub sources: EstimateSources<'services>,
}

#[async_trait]
impl ReservationCostSource for RequestCosts<'_> {
    async fn input_tokens(
        &self,
        request: &EstimateRequest<'_>,
    ) -> Result<IndexMap<String, usize>, ReservationCostError> {
        count_request_input_tokens(request, &self.sources).await
    }

    async fn max_cost(
        &self,
        request: &EstimateRequest<'_>,
        input_token_counts: Option<&IndexMap<String, usize>>,
    ) -> Result<Option<Decimal>, ReservationCostError> {
        estimate_request_max_cost(request, &self.sources, input_token_counts).await
    }

    async fn input_cost(
        &self,
        request: &EstimateRequest<'_>,
        input_token_counts: Option<&IndexMap<String, usize>>,
    ) -> Result<Option<Decimal>, ReservationCostError> {
        estimate_request_input_cost(request, &self.sources, input_token_counts).await
    }
}

// B3004's admit over the server's spend counter backend
pub struct CounterAdmission<'counters, C: AdmissionCounters> {
    pub counters: &'counters C,
}

#[async_trait]
impl<C: AdmissionCounters> ReservationAdmission for CounterAdmission<'_, C> {
    async fn admit(
        &self,
        request: &AdmissionRequest<'_>,
        applied_entries: &mut Vec<ReservationEntry>,
    ) -> Result<Admission, AdmissionError> {
        admit(self.counters, request, applied_entries).await
    }
}

// B3003's batch scope and partial release over the same backend
pub struct CounterSettlement<'counters, C: SettlementCounters> {
    pub counters: &'counters C,
}

#[async_trait]
impl<C: SettlementCounters> ReservationSettlement for CounterSettlement<'_, C> {
    fn enter_scope(&self, counter_keys: &BTreeSet<String>) -> Box<dyn ScopeHold + Send + '_> {
        self.counters.enter_counters_batch_scope(counter_keys)
    }

    async fn release_entries(
        &self,
        entries: &mut [ReservationEntry],
        default_reserved_cost: Decimal,
    ) {
        release_entries_best_effort(self.counters, entries, default_reserved_cost).await;
    }
}

// B3015's refresh_spend_counter_ttl over the spend counter writes the server built
pub struct SpendCounterLeases<Layers, Rows>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    pub writes: Arc<SpendCounterWrites<Layers, Rows>>,
}

#[async_trait]
impl<Layers, Rows> ReservationLeases for SpendCounterLeases<Layers, Rows>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + 'static,
    Rows: SpendCounterRows,
{
    async fn refresh(&self, counter_key: &str) {
        let _refreshed = refresh_spend_counter_ttl(self.writes.as_ref(), counter_key).await;
    }
}

// spend_counter_cache.redis_cache: no Redis backend or no positive TTL means no renewal
#[must_use]
pub fn renewal_from_redis<'tasks, Layers, Rows>(
    writes: Arc<SpendCounterWrites<Layers, Rows>>,
    redis: Option<&RedisCache>,
    request_timeout: Duration,
    tasks: &'tasks TaskTracker,
) -> Option<LeaseRenewal<'tasks>>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + 'static,
    Rows: SpendCounterRows,
{
    let redis = redis?;
    let interval = lease_interval(redis.default_ttl)?;
    Some(LeaseRenewal {
        leases: Arc::new(SpendCounterLeases { writes }),
        interval,
        request_timeout,
        tasks,
    })
}

// BaseCache.default_ttl halved, in milliseconds so an odd TTL keeps its half second
#[must_use]
pub fn lease_interval(default_ttl: i64) -> Option<Duration> {
    let ttl = u64::try_from(default_ttl).ok()?;
    if ttl == 0 {
        return None;
    }
    let half_millis = ttl.checked_mul(500)?;
    Some(Duration::from_millis(half_millis))
}
