use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    GetCurrentSpend,
    ReadSpendCounterCacheValue,
}

// LiteLLM opens a spend counter batch only for a Redis-backed request, so "none" means no active batch
#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BatchMode {
    None,
    Miss,
    Amount,
}

// A bool on the wire; the newtype keeps struct_excessive_bools off the case input.
#[derive(Deserialize, Clone, Copy)]
#[serde(transparent)]
pub(crate) struct Flag(pub(crate) bool);

#[derive(Deserialize, Clone)]
pub(crate) struct RowsInput {
    pub(crate) primary: Option<Decimal>,
    pub(crate) end_user: Option<Decimal>,
    pub(crate) window: Option<Decimal>,
    pub(crate) fails: Flag,
    // a budget reset that commits during the entity read writes this post-reset floor marker
    pub(crate) reset_marker: Option<Decimal>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    pub(crate) op: Operation,
    pub(crate) counter_key: String,
    pub(crate) fallback_spend: Decimal,
    pub(crate) max_budget: Option<Decimal>,
    pub(crate) window_entity_type: Option<String>,
    pub(crate) window_entity_id: Option<String>,
    pub(crate) window_duration: Option<String>,
    pub(crate) window_start: Option<String>,
    pub(crate) fallback_authoritative: Flag,
    pub(crate) strict: Flag,
    pub(crate) redis: Flag,
    pub(crate) redis_get: Option<String>,
    pub(crate) redis_fails: Flag,
    pub(crate) set_max: Option<Decimal>,
    pub(crate) set_max_fails: Flag,
    pub(crate) batch: BatchMode,
    pub(crate) batch_amount: Decimal,
    pub(crate) memory: Option<String>,
    pub(crate) marker: Option<Decimal>,
    pub(crate) rows: Option<RowsInput>,
}
