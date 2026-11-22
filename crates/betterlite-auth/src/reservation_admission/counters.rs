use std::time::SystemTime;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::budget_counter::CounterEntityType;
use crate::reservation_settlement::counters::SettlementCounters;
use crate::reservation_settlement::error::SettlementError;
use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_writes::error::SpendWriteError;

// _ensure_window_spend_counter_initialized's arguments, with the counter's options resolved by the caller
#[derive(Clone, Copy)]
pub struct WindowSeed<'seed> {
    pub counter_key: &'seed str,
    pub entity_type: &'seed CounterEntityType,
    pub entity_id: &'seed str,
    pub window_duration: Option<&'seed str>,
    pub window_start: SystemTime,
}

// the reservation-only counter operations on top of the settlement ones
#[async_trait]
pub trait AdmissionCounters: SettlementCounters {
    // _ensure_spend_counter_initialized: warm the counter or seed it from its source cache row
    async fn seed_source(
        &self,
        counter_key: &str,
        source_cache_key: &str,
    ) -> Result<(), SpendWriteError>;

    // _ensure_window_spend_counter_initialized: false when the window spend could not be loaded
    async fn seed_window(&self, seed: WindowSeed<'_>) -> Result<bool, SpendWriteError>;

    // _get_current_counter_value: get_current_spend with no max_budget verdict, so the estimate is the answer
    async fn current_spend(&self, counter_key: &str, fallback_spend: Decimal) -> Decimal;

    // run_spend_counter_pipeline: each counter's value after the increment, None for one the pipeline skipped
    async fn reserve_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Option<Decimal>>, SettlementError>;
}
