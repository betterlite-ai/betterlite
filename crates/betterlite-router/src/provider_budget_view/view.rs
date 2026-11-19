use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::provider_budget_view::ProviderBudgetConfig;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::{Decimal, RoundingStrategy};

use super::cache::SpendCache;
use super::error::ProviderBudgetViewError;

const MICROSECONDS_PER_SECOND: i64 = 1_000_000;

// The two budget_limiter provider reads behind GET /provider/budgets, free of the higher budget policy
#[async_trait]
pub trait ProviderBudgetReads: Send + Sync {
    async fn current_provider_spend(
        &self,
        provider: &str,
    ) -> Result<Option<Decimal>, ProviderBudgetViewError>;

    async fn provider_budget_reset_at(
        &self,
        provider: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, ProviderBudgetViewError>;
}

pub struct ProviderBudgetView {
    pub provider_budget_config: Option<IndexMap<String, ProviderBudgetConfig>>,
    pub redis: Option<Arc<dyn SpendCache>>,
    pub dual: Arc<dyn SpendCache>,
}

impl ProviderBudgetView {
    // _get_budget_config_for_provider plus the provider_spend key, which renders a missing duration as None
    fn spend_key(&self, provider: &str) -> Option<String> {
        let budget = self.provider_budget_config.as_ref()?.get(provider)?;
        let duration = budget.budget_duration.as_deref().unwrap_or("None");
        Some(format!("provider_spend:{provider}:{duration}"))
    }
}

#[async_trait]
impl ProviderBudgetReads for ProviderBudgetView {
    async fn current_provider_spend(
        &self,
        provider: &str,
    ) -> Result<Option<Decimal>, ProviderBudgetViewError> {
        let Some(key) = self.spend_key(provider) else {
            return Ok(None);
        };
        let cache = match &self.redis {
            Some(redis) => redis.as_ref(),
            None => self.dual.as_ref(),
        };
        let spend = cache.spend(&key).await?;
        Ok(Some(spend.unwrap_or(Decimal::ZERO)))
    }

    async fn provider_budget_reset_at(
        &self,
        provider: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, ProviderBudgetViewError> {
        let Some(key) = self.spend_key(provider) else {
            return Ok(None);
        };
        let cache = match &self.redis {
            Some(redis) => redis.as_ref(),
            None => self.dual.as_ref(),
        };
        let Some(seconds) = cache.expiry_seconds(&key).await else {
            return Ok(None);
        };
        let micros = expiry_microseconds(seconds, &key)?;
        let delta = chrono::Duration::try_microseconds(micros)
            .ok_or_else(|| ProviderBudgetViewError::Expiry { key: key.clone() })?;
        let moment = now
            .checked_add_signed(delta)
            .ok_or_else(|| ProviderBudgetViewError::Expiry { key })?;
        Ok(Some(isoformat(moment)))
    }
}

// timedelta(seconds=...) rounds a fractional expiry to the nearest microsecond, half to even
fn expiry_microseconds(seconds: f64, key: &str) -> Result<i64, ProviderBudgetViewError> {
    Decimal::from_f64_retain(seconds)
        .and_then(|value| value.checked_mul(Decimal::from(MICROSECONDS_PER_SECOND)))
        .and_then(|value| {
            value
                .round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven)
                .to_i64()
        })
        .ok_or_else(|| ProviderBudgetViewError::Expiry {
            key: key.to_owned(),
        })
}

// datetime.isoformat prints six fractional digits only when the microsecond is not zero
fn isoformat(moment: DateTime<Utc>) -> String {
    if moment.timestamp_subsec_micros() == 0 {
        return moment.format("%Y-%m-%dT%H:%M:%S+00:00").to_string();
    }
    moment.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
}
