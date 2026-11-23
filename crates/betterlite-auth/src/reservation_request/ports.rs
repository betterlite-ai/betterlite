use std::collections::BTreeSet;

use async_trait::async_trait;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::budget_counter::BudgetCounter;
use crate::reservation_admission::engine::{Admission, AdmissionRequest};
use crate::reservation_admission::error::AdmissionError;
use crate::reservation_cost::EstimateRequest;
use crate::reservation_cost::error::ReservationCostError;
use crate::reservation_counters::CounterInput;
use crate::reservation_counters::error::ReservationCounterError;
use crate::reservation_settlement::counters::ScopeHold;
use crate::reservation_settlement::entry::ReservationEntry;

// B3002's _get_budget_counters, narrowed to the counter list the reservation reserves against
#[async_trait]
pub trait ReservationCounterSource: Send + Sync {
    async fn counters(
        &self,
        input: &CounterInput<'_>,
    ) -> Result<Vec<BudgetCounter>, ReservationCounterError>;
}

// B3001's three estimates, grouped so the counts a request computed are the ones its costs read
#[async_trait]
pub trait ReservationCostSource: Send + Sync {
    async fn input_tokens(
        &self,
        request: &EstimateRequest<'_>,
    ) -> Result<IndexMap<String, usize>, ReservationCostError>;

    async fn max_cost(
        &self,
        request: &EstimateRequest<'_>,
        input_token_counts: Option<&IndexMap<String, usize>>,
    ) -> Result<Option<Decimal>, ReservationCostError>;

    async fn input_cost(
        &self,
        request: &EstimateRequest<'_>,
        input_token_counts: Option<&IndexMap<String, usize>>,
    ) -> Result<Option<Decimal>, ReservationCostError>;
}

// B3004's admit: initialize the counters, then charge them through the over-budget policy
#[async_trait]
pub trait ReservationAdmission: Send + Sync {
    async fn admit(
        &self,
        request: &AdmissionRequest<'_>,
        applied_entries: &mut Vec<ReservationEntry>,
    ) -> Result<Admission, AdmissionError>;
}

// B3003's batch scope and the best-effort release of the entries a failed reservation applied
#[async_trait]
pub trait ReservationSettlement: Send + Sync {
    fn enter_scope(&self, counter_keys: &BTreeSet<String>) -> Box<dyn ScopeHold + Send + '_>;

    async fn release_entries(
        &self,
        entries: &mut [ReservationEntry],
        default_reserved_cost: Decimal,
    );
}

// B3015's refresh_spend_counter_ttl: the boolean only says whether the key still existed
#[async_trait]
pub trait ReservationLeases: Send + Sync {
    async fn refresh(&self, counter_key: &str);
}
