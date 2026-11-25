use std::collections::BTreeSet;
use std::convert::Infallible;
use std::error::Error as StdError;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::reservation_settlement::adjust::AdjustmentMode;
use betterlite_auth::reservation_settlement::counters::{ScopeHold, SettlementCounters};
use betterlite_auth::reservation_settlement::error::SettlementError;
use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_auth::reservation_settlement::reconcile::ReservationFinality;
use betterlite_auth::spend_counter_batch::scope::SpendCounterScope;
use betterlite_auth::spend_increments::error::SpendIncrementError;
use betterlite_auth::spend_increments::{
    SpendIncrementInput, SpendIncrementServices, increment_spend_counters,
};
use betterlite_auth::spend_reseed::cache::dual::ReseedCacheError;
use betterlite_auth::spend_reseed::cache::{ReseedCache, WindowReseedCache};
use betterlite_auth::spend_reseed::rows::SpendCounterRows;
use betterlite_auth::spend_writes::layers::CounterLayers;
use betterlite_db::spend_log_storage::batch_claim::BatchClaimTable;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use betterlite_db::spend_writer::DbSpendUpdateWriter;
use betterlite_db::spend_writer::request::{SpendPayloadSource, SpendUpdateRequest};
use betterlite_models::savings_pricing::context::SavingsPricingContext;
use betterlite_redis::redis_cache::RedisCache;
use rust_decimal::Decimal;

// B8481 maps each method onto B3003 over the one spend counter cache, so this coordinator never reaches the budget services
#[async_trait]
pub trait ReservationSettlement: Send + Sync {
    async fn reconcile(
        &self,
        reservation: &ReservationHandle,
        actual_cost: Decimal,
        finality: ReservationFinality,
        mode: AdjustmentMode,
    ) -> Result<(), SettlementError>;

    async fn release(&self, reservation: &ReservationHandle) -> Result<(), SettlementError>;

    async fn invalidate(&self, reservation: &ReservationHandle) -> Result<(), SettlementError>;
}

// spend_counter_batch_scope: the hold keeps one MGET answering every counter read until it drops
pub trait CounterBatchScope: Send + Sync {
    fn open(&self, counter_keys: &BTreeSet<String>) -> Box<dyn ScopeHold + Send + '_>;
}

// the request-owned batch B1718 builds, with the Redis tier its MGET reads
pub struct RequestCounterBatch<'request> {
    pub scope: &'request SpendCounterScope,
    pub redis: Option<&'request Arc<RedisCache>>,
}

impl CounterBatchScope for RequestCounterBatch<'_> {
    fn open(&self, counter_keys: &BTreeSet<String>) -> Box<dyn ScopeHold + Send + '_> {
        Box::new(self.scope.enter(self.redis, counter_keys))
    }
}

// db_spend_update_writer.update_database: true while this request still owns its charge
pub trait SpendWrite: Sync {
    type Error: StdError + Send + Sync + 'static;

    fn update_database(&self) -> impl Future<Output = Result<bool, Self::Error>> + Send;
}

// B070's writer reports its own failures and still answers true, so this adapter never yields the write error
pub struct DatabaseSpendWrite<'call, 'work, 'request, Table, Prices, Payloads>
where
    Table: BatchClaimTable + ?Sized,
{
    pub writer: &'call DbSpendUpdateWriter<'work, Table>,
    pub payloads: &'call Payloads,
    pub request: &'call SpendUpdateRequest<'request, Prices>,
}

impl<Table, Prices, Payloads> SpendWrite for DatabaseSpendWrite<'_, '_, '_, Table, Prices, Payloads>
where
    Table: BatchClaimTable + ?Sized,
    Prices: SavingsPricingContext,
    Payloads: SpendPayloadSource + Sync,
{
    type Error = Infallible;

    async fn update_database(&self) -> Result<bool, Infallible> {
        Ok(self
            .writer
            .update_database(self.payloads, self.request)
            .await)
    }
}

// no + Send: awaiting B8593's seeding chain hits rustc's dyn-batch generalization false positive (rust-lang/rust#130113)
pub trait SpendCounterIncrement: Sync {
    fn increment(
        &self,
        input: &SpendIncrementInput<'_>,
    ) -> impl Future<Output = Result<(), SpendIncrementError>>;
}

impl<Counters, Layers, Rows> SpendCounterIncrement
    for SpendIncrementServices<'_, Counters, Layers, Rows>
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    async fn increment(&self, input: &SpendIncrementInput<'_>) -> Result<(), SpendIncrementError> {
        increment_spend_counters(input, self).await
    }
}
