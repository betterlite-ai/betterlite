use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;

use super::error::IncrementError;
use super::queue::QueuedIncrement;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetConfig {
    pub max_budget: Option<Decimal>,
    pub budget_duration: Option<Duration>,
}

#[async_trait]
pub trait WindowCache: Send + Sync {
    async fn read_start_time(&self, key: &str) -> Result<Option<f64>, IncrementError>;

    async fn write_start_time(
        &self,
        key: &str,
        start_time: f64,
        ttl: Duration,
    ) -> Result<(), IncrementError>;

    async fn reset_spend(
        &self,
        key: &str,
        cost: Decimal,
        ttl: Duration,
    ) -> Result<(), IncrementError>;

    async fn increment_spend(
        &self,
        key: &str,
        cost: Decimal,
        ttl: Duration,
    ) -> Result<Decimal, IncrementError>;

    async fn write_increments(&self, operations: &[QueuedIncrement]) -> bool;
}

#[async_trait]
pub trait WindowRedis: Send + Sync {
    async fn read_start_time(&self, key: &str) -> Result<Option<f64>, IncrementError>;

    async fn write_start_time(&self, key: &str, start_time: f64, ttl: Duration);

    async fn write_spend(&self, key: &str, cost: Decimal, ttl: Duration);

    async fn write_increments(&self, operations: &[QueuedIncrement]) -> bool;
}
