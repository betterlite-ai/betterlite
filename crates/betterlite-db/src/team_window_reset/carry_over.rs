use rust_decimal::Decimal;

use crate::budget_reset_policy::rollover::{RolloverPolicy, carried_spend};

use super::cache::WindowCounterCache;

// _window_carried_spend: per-window spend lives only in the counter, so the overage above the window cap is read before the reset overwrites it; an unreadable or non-numeric counter resets to a plain zero.
pub(crate) async fn window_carried_spend<Cache>(
    max_budget: Option<Decimal>,
    counter_key: &str,
    policy: RolloverPolicy,
    cache: &Cache,
) -> Decimal
where
    Cache: WindowCounterCache,
{
    if !policy.enabled {
        return Decimal::ZERO;
    }
    let Some(cap) = max_budget else {
        return Decimal::ZERO;
    };
    let current = match cache.read(counter_key).await {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                counter = %counter_key,
                error = %error,
                "Failed to read the window spend counter for rollover; resetting it to zero"
            );
            return Decimal::ZERO;
        }
    };
    carried_spend(current, Some(cap))
}

mod tests;
