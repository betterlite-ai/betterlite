use async_trait::async_trait;
use rust_decimal::Decimal;

use super::error::ProviderBudgetViewError;

// The read side of whichever cache holds provider spend. A failure stays a failure so Redis errors never read as local spend.
#[async_trait]
pub trait SpendCache: Send + Sync {
    async fn spend(&self, key: &str) -> Result<Option<Decimal>, ProviderBudgetViewError>;

    // LiteLLM's TTL reads answer None where the driver raised, so this port has no error to report
    async fn expiry_seconds(&self, key: &str) -> Option<f64>;
}
