use std::collections::BTreeMap;

use super::hash_tag_group::group_keys_by_hash_tag;
use super::no_retry::read_group_values_once;
use super::{CounterValues, WindowStore, WindowStoreError};
use crate::limiter_records::CacheCounterValue;

pub async fn read_counter_values_from_redis(
    store: &WindowStore,
    keys: &[String],
) -> Result<CounterValues, WindowStoreError> {
    let Some(script) = store.counter_read_script.as_deref() else {
        return Ok(Vec::new());
    };
    let groups = group_keys_by_hash_tag(store, keys);
    let group_values = read_group_values_once(script, &groups).await?;
    Ok(values_by_key_order(keys, &groups, group_values))
}

#[must_use]
pub fn values_by_key_order(
    keys: &[String],
    groups: &[(String, Vec<String>)],
    group_values: Vec<CounterValues>,
) -> CounterValues {
    let mut by_key: BTreeMap<&str, Option<CacheCounterValue>> = BTreeMap::new();
    for ((_tag, group_keys), values) in groups.iter().zip(group_values) {
        for (key, value) in group_keys.iter().zip(values) {
            by_key.insert(key.as_str(), value);
        }
    }
    keys.iter()
        .map(|key| by_key.get(key.as_str()).cloned().flatten())
        .collect()
}

mod tests;
