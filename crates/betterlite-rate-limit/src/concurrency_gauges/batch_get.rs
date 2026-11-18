use super::cache::{GaugeCache, GaugeCacheValue};

mod tests;

pub(super) async fn batch_gauge_values(
    cache: &dyn GaugeCache,
    keys: &[String],
) -> Vec<Option<GaugeCacheValue>> {
    match cache.batch_get(keys, true).await {
        Some(values) => values,
        None => vec![None; keys.len()],
    }
}
