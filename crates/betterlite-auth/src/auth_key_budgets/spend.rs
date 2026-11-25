use async_trait::async_trait;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::current_spend::{CurrentSpendError, SpendQuery};

// the cross-pod read the hard and window checks perform; a read that cannot be verified keeps the 503 the reader raises
pub type SpendReadError = CurrentSpendError<SpendReadFailure>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SpendReadFailure {
    pub message: String,
}

#[async_trait]
pub trait SpendReader: Send + Sync {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError>;
}
