use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use crate::model_budget_values::amount_text::amount_from_text;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::WindowReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_reseed::values::{ReseedCacheValue, cache_amount};
use crate::spend_reseed::windows::WindowReseed;

use super::SpendCounterWrites;
use super::error::SpendWriteError;
use super::increment::increment_spend_counter;
use super::layers::CounterLayers;
use super::warm::is_spend_counter_cache_warm;

const SPEND_FIELD: &str = "spend";

pub async fn ensure_counter_seeded<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    counter_key: &str,
    source_cache_keys: &[&str],
) -> Result<(), SpendWriteError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    if is_spend_counter_cache_warm(writes, counter_key).await? {
        return Ok(());
    }
    let seeded = writes
        .reseed
        .coalesced(&*writes.layers, &writes.batch, counter_key, true)
        .await?;
    if seeded.is_some() {
        return Ok(());
    }
    // the database holds no row, so the auth cache's last known spend stands in for the floor
    let base_spend = get_source_cache_base_spend(writes, source_cache_keys).await?;
    if base_spend > Decimal::ZERO {
        let _counted = increment_spend_counter(writes, counter_key, base_spend).await?;
    }
    Ok(())
}

pub async fn ensure_window_seeded<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    request: &WindowReseed<'_>,
) -> Result<bool, SpendWriteError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache,
    Rows: SpendCounterRows,
{
    if is_spend_counter_cache_warm(writes, request.counter_key).await? {
        return Ok(true);
    }
    let window_spend = writes
        .reseed
        .coalesced_window(&*writes.layers, &writes.batch, request)
        .await?;
    if window_spend.is_none() {
        tracing::warn!(
            counter_key = request.counter_key,
            "Skipping cold spend counter seed because window spend could not be loaded"
        );
        return Ok(false);
    }
    Ok(true)
}

pub async fn get_source_cache_base_spend<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    source_cache_keys: &[&str],
) -> Result<Decimal, SpendWriteError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    for cache_key in source_cache_keys {
        let Some(source) = writes.sources.cached(cache_key).await else {
            continue;
        };
        let Json::Object(fields) = &source else {
            return Ok(Decimal::ZERO);
        };
        let Some(spend) = fields.get(SPEND_FIELD) else {
            return Ok(Decimal::ZERO);
        };
        return source_amount(spend);
    }
    Ok(Decimal::ZERO)
}

// float(source.get("spend", 0.0) or 0.0), with the auth cache's typed rows read as their JSON fields
fn source_amount(spend: &Json) -> Result<Decimal, SpendWriteError> {
    let value = match spend {
        Json::Null | Json::Bool(false) => ReseedCacheValue::Number(Decimal::ZERO),
        Json::List(items) if items.is_empty() => ReseedCacheValue::Number(Decimal::ZERO),
        Json::Object(fields) if fields.is_empty() => ReseedCacheValue::Number(Decimal::ZERO),
        Json::Text(text) if text.is_empty() => ReseedCacheValue::Number(Decimal::ZERO),
        Json::Bool(flag) => ReseedCacheValue::Boolean(*flag),
        Json::Number(number) => amount_from_text(&number.to_string())
            .map_or(ReseedCacheValue::Unreadable, ReseedCacheValue::Number),
        Json::Text(text) => ReseedCacheValue::Text(text.clone()),
        Json::List(_) | Json::Object(_) => ReseedCacheValue::Unreadable,
    };
    Ok(cache_amount(&value)?)
}
