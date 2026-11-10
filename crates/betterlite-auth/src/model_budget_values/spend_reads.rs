pub mod error;
mod tests;

use std::collections::BTreeMap;
use std::error::Error;

use rust_decimal::Decimal;

use super::amount_text::amount_from_text;
use error::SpendReadError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CachedSpend {
    Number(Decimal),
    Text(String),
    Unreadable,
}

pub trait SpendCache {
    type Error: Error + 'static;

    // Ok(None) means no Redis is configured; a key Redis does not hold is left out of the map
    fn redis_batch_get(
        &self,
        keys: &[String],
    ) -> impl Future<Output = Result<Option<BTreeMap<String, CachedSpend>>, Self::Error>> + Send;

    // None means the in-memory batch read failed; the list is index-aligned with the keys otherwise
    fn memory_batch_get(
        &self,
        keys: &[String],
    ) -> impl Future<Output = Option<Vec<Option<CachedSpend>>>> + Send;
}

pub async fn current_window_spends<C>(
    cache: &C,
    spend_keys: &[String],
) -> Result<Vec<Decimal>, SpendReadError<C::Error>>
where
    C: SpendCache + Sync,
{
    let redis_values = cache
        .redis_batch_get(spend_keys)
        .await
        .map_err(|source| SpendReadError::Redis { source })?;
    if let Some(held) = redis_values {
        return Ok(spend_keys
            .iter()
            .map(|key| spend_amount(held.get(key)))
            .collect());
    }
    match cache.memory_batch_get(spend_keys).await {
        Some(batched) if batched.len() == spend_keys.len() => Ok(batched
            .iter()
            .map(|spend| spend_amount(spend.as_ref()))
            .collect()),
        Some(_) | None => Ok(vec![Decimal::ZERO; spend_keys.len()]),
    }
}

fn spend_amount(value: Option<&CachedSpend>) -> Decimal {
    match value {
        Some(CachedSpend::Number(amount)) => *amount,
        Some(CachedSpend::Text(text)) => amount_from_text(text.trim()).unwrap_or(Decimal::ZERO),
        Some(CachedSpend::Unreadable) | None => Decimal::ZERO,
    }
}
