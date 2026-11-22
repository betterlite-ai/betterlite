use std::collections::BTreeSet;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_counter_batch::scope::SpendCounterScopeGuard;

use super::error::{CounterReadError, SettlementError};

// the hold _counters_batch_scope keeps alive while one MGET serves every counter read behind it
pub trait ScopeHold {}

impl ScopeHold for SpendCounterScopeGuard<'_> {}

// the spend counter operations settlement reaches, injected so the server's cache and the tests share one path
#[async_trait]
pub trait SettlementCounters: Send + Sync {
    fn enter_counters_batch_scope(
        &self,
        counter_keys: &BTreeSet<String>,
    ) -> Box<dyn ScopeHold + Send + '_>;

    async fn read_cache_value(
        &self,
        counter_key: &str,
    ) -> Result<Option<Decimal>, CounterReadError>;

    async fn reseed_from_db(&self, counter_key: &str) -> bool;

    async fn increment_counter(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<(), SettlementError>;

    async fn increment_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<(), SettlementError>;

    async fn invalidate(&self, counter_key: &str);
}
