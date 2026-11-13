use super::{CounterValues, WindowStore};

pub async fn batch_get_counter_values(
    store: &WindowStore,
    keys: &[String],
    local_only: bool,
) -> Option<CounterValues> {
    let values = store.cache.batch_get_cache(keys, local_only).await?;
    if values.len() != keys.len() {
        return None;
    }
    Some(values)
}

mod tests;
