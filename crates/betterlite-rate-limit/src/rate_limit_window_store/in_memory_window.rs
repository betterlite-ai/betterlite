use super::counter_value::counter_as_int;
use super::{CounterValues, WindowStore, WindowStoreError};
use crate::limiter_records::{CacheCounterValue, sibling_counter_keys};

const INCREMENT: i64 = 1;

pub async fn in_memory_cache_sliding_window(
    store: &WindowStore,
    keys: &[String],
    now_int: i64,
    window_size: i64,
) -> Result<CounterValues, WindowStoreError> {
    let _guard = store.check_and_increment_lock.lock().await;
    let mut results = Vec::with_capacity(keys.len());
    for pair in keys.chunks(2) {
        let (Some(window_key), Some(counter_key)) = (pair.first(), pair.get(1)) else {
            return Err(WindowStoreError::UnpairedKeys { count: keys.len() });
        };
        let window_start = store.cache.get_cache(window_key, true).await;
        let expired = match window_start.as_ref() {
            Some(value) => now_int.saturating_sub(counter_as_int(value)?) >= window_size,
            None => true,
        };
        if expired {
            let values = reset_window(store, window_key, counter_key, now_int, window_size).await;
            results.extend(values);
        } else {
            let values = increment_counter(store, counter_key, window_start, window_size).await?;
            results.extend(values);
        }
    }
    Ok(results)
}

async fn reset_window(
    store: &WindowStore,
    window_key: &str,
    counter_key: &str,
    now_int: i64,
    window_size: i64,
) -> CounterValues {
    for sibling in <[String; 2]>::from(sibling_counter_keys(window_key)) {
        store
            .cache
            .set_cache(&sibling, CacheCounterValue::Int(0), window_size, true)
            .await;
    }
    let started = CacheCounterValue::Text(now_int.to_string());
    store
        .cache
        .set_cache(window_key, started.clone(), window_size, true)
        .await;
    store
        .cache
        .set_cache(
            counter_key,
            CacheCounterValue::Int(INCREMENT),
            window_size,
            true,
        )
        .await;
    vec![Some(started), Some(CacheCounterValue::Int(INCREMENT))]
}

async fn increment_counter(
    store: &WindowStore,
    counter_key: &str,
    window_start: Option<CacheCounterValue>,
    window_size: i64,
) -> Result<CounterValues, WindowStoreError> {
    let current = match store.cache.get_cache(counter_key, true).await {
        Some(value) => counter_as_int(&value)?,
        None => 0,
    };
    let new_counter = current
        .checked_add(INCREMENT)
        .ok_or(WindowStoreError::CounterOverflow)?;
    store
        .cache
        .set_cache(
            counter_key,
            CacheCounterValue::Int(new_counter),
            window_size,
            true,
        )
        .await;
    Ok(vec![
        window_start,
        Some(CacheCounterValue::Int(new_counter)),
    ])
}

mod tests;
