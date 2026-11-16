use std::fmt::Display;

use rust_decimal::Decimal;

use crate::model_budget_values::amount_text::amount_from_text;
use crate::model_budget_values::spend_reads::CachedSpend;

use super::SessionBudgetHandler;
use super::cache::SessionBudgetCache;

mod tests;

// _get_current_spend: the Redis value wins, and an exception or a non-number falls back to the local cache
pub(crate) async fn current_spend<C>(handler: &SessionBudgetHandler<C>, key: &str) -> Decimal
where
    C: SessionBudgetCache,
{
    if handler.cache.redis_configured() {
        let read = handler.cache.redis_spend(key).await;
        if let Some(spend) = redis_spend(read) {
            return spend;
        }
    }
    handler
        .cache
        .memory_spend(key)
        .await
        .as_ref()
        .and_then(spend_amount)
        .unwrap_or(Decimal::ZERO)
}

// _increment_spend: the Lua script first, and the in-memory read-add-write when it is absent or fails
pub(crate) async fn add_spend<C>(handler: &SessionBudgetHandler<C>, key: &str, amount: Decimal)
where
    C: SessionBudgetCache,
{
    let Some(script) = handler.script.as_ref() else {
        add_in_memory(handler, key, amount).await;
        return;
    };
    let increment = handler
        .cache
        .run_increment_script(script, key, amount, handler.ttl)
        .await;
    if let Err(failure) = increment {
        tracing::warn!(
            "MaxBudgetPerSessionHandler: Redis INCRBYFLOAT failed, falling back to in-memory: {failure}"
        );
        add_in_memory(handler, key, amount).await;
    }
}

async fn add_in_memory<C>(handler: &SessionBudgetHandler<C>, key: &str, amount: Decimal)
where
    C: SessionBudgetCache,
{
    let current = handler.cache.memory_spend(key).await;
    let spent = current
        .as_ref()
        .and_then(spend_amount)
        .unwrap_or(Decimal::ZERO);
    let next = spent.checked_add(amount).unwrap_or(Decimal::MAX);
    handler
        .cache
        .memory_set_spend(key, CachedSpend::Number(next), handler.ttl)
        .await;
}

// _get_current_spend's Redis branch: it fails over to the local cache instead of failing the request
fn redis_spend<E>(read: Result<Option<CachedSpend>, E>) -> Option<Decimal>
where
    E: Display,
{
    match read {
        Ok(Some(value)) => spend_amount(&value).map_or_else(
            || {
                tracing::warn!(
                    "MaxBudgetPerSessionHandler: Redis GET returned a value that is not a number, falling back to in-memory"
                );
                None
            },
            Some,
        ),
        Ok(None) => Some(Decimal::ZERO),
        Err(failure) => {
            tracing::warn!(
                "MaxBudgetPerSessionHandler: Redis GET failed, falling back to in-memory: {failure}"
            );
            None
        }
    }
}

fn spend_amount(value: &CachedSpend) -> Option<Decimal> {
    match value {
        CachedSpend::Number(amount) => Some(*amount),
        CachedSpend::Text(text) => amount_from_text(text.trim()),
        CachedSpend::Unreadable => None,
    }
}
