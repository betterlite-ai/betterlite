use super::cache::{GaugeCache, GaugeCacheValue};

mod tests;

pub(super) async fn batch_gauge_values(
    cache: &dyn GaugeCache,
    keys: &[String],
) -> Vec<Option<GaugeCacheValue>> {
    let values = cache.batch_get(keys, true).await;
    values.unwrap_or_else(|| vec![None; keys.len()])
}
