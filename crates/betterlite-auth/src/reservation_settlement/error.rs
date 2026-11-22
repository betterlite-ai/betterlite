use thiserror::Error;

use crate::spend_writes::error::SpendWriteError;

#[derive(Debug, Error)]
pub enum SettlementError {
    #[error("the reservation cost is outside the decimal range")]
    Overflow,
    // LiteLLM's RuntimeError from _set_reserved_entries_actual_cost with reseed_on_inconsistent False
    #[error("cannot resize a budget reservation against inconsistent counter {counter_key}")]
    InconsistentCounter { counter_key: String },
    #[error(transparent)]
    Write(#[from] SpendWriteError),
}

// LiteLLM's TypeError and ValueError from read_spend_counter_cache_value, which only its float parsing raises
#[derive(Debug, Error)]
pub enum CounterReadError {
    #[error("the spend counter {counter_key} does not hold a numeric value")]
    NotNumeric { counter_key: String },
}
