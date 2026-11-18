use std::collections::{BTreeMap, HashMap};
use std::str::FromStr as _;
use std::sync::PoisonError;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::cache_primitives::decode::CachedValue;
use rust_decimal::Decimal;

use crate::budget_increment::dual::DualWindowCache;
use crate::budget_increment::error::IncrementError;
use crate::budget_increment::redis_cache_window::RedisCacheWindow;
use crate::budget_increment::window::WindowRedis;
use crate::budget_windows::keys::{deployment_keys, provider_keys, tag_keys};
use crate::budget_windows::service::BudgetWindows;

const SPEND_FIELD: &str = "spend";

// _flush_increments_then_copy_redis_spend reads every configured spend key back from the shared store
#[must_use]
pub fn spend_keys(windows: &BudgetWindows) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(providers) = &windows.provider {
        for (provider, budget) in providers {
            keys.push(provider_keys(provider, &budget.period).spend);
        }
    }
    if let Some(deployments) = &windows.deployment {
        for (model_id, budget) in deployments {
            keys.push(deployment_keys(model_id, &budget.period).spend);
        }
    }
    if let Some(tags) = &windows.tag {
        for (tag, budget) in tags {
            keys.push(tag_keys(tag, &budget.period).spend);
        }
    }
    keys
}

// what a batch read answered: a value per requested key, or the whole read failed
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpendRead {
    Values(BTreeMap<String, Option<Decimal>>),
    Failed,
}

// the batch read the copy needs from a shared store; a failed read is not an empty spend
#[async_trait]
pub trait SpendRedis: Send + Sync {
    async fn read_spends(&self, keys: &[String]) -> SpendRead;
}

#[async_trait]
impl SpendRedis for RedisCacheWindow {
    async fn read_spends(&self, keys: &[String]) -> SpendRead {
        let requested: Vec<Option<&str>> = keys.iter().map(|key| Some(key.as_str())).collect();
        match self.cache.async_batch_get_cache(&requested).await {
            Ok(values) => decode_spends(keys, &values),
            Err(_error) => SpendRead::Failed,
        }
    }
}

// the cache swallows a failed or undecodable batch into an empty map, and only an empty request is empty
pub(super) fn decode_spends(
    keys: &[String],
    values: &HashMap<String, Option<CachedValue>>,
) -> SpendRead {
    if values.is_empty() && !keys.is_empty() {
        return SpendRead::Failed;
    }
    let mut spends = BTreeMap::new();
    for key in keys {
        let spend = match values.get(key) {
            Some(Some(CachedValue::Number(number))) => {
                let text = number.to_string();
                let Ok(spend) = Decimal::from_str(&text) else {
                    return SpendRead::Failed;
                };
                Some(spend)
            }
            // float() reads a numeric string, so a text value counts and any other shape fails the copy
            Some(Some(CachedValue::Text(text))) => {
                let Ok(spend) = Decimal::from_str(text) else {
                    return SpendRead::Failed;
                };
                Some(spend)
            }
            Some(Some(_)) => return SpendRead::Failed,
            Some(None) | None => None,
        };
        spends.insert(key.clone(), spend);
    }
    SpendRead::Values(spends)
}

// the copy writes the merged value into the memory cache and leaves the shared store alone
pub fn set_local_spend<R: WindowRedis>(
    cache: &DualWindowCache<R>,
    key: &str,
    spend: Decimal,
) -> Result<(), IncrementError> {
    let encoded = CacheValue::Object(BTreeMap::from([(
        SPEND_FIELD.to_owned(),
        CacheValue::Text(spend.to_string()),
    )]));
    let mut memory = cache.memory.lock().unwrap_or_else(PoisonError::into_inner);
    // as in_memory_cache.async_set_cache does, a set without a ttl keeps a live window expiry
    memory.set(key, encoded, None)?;
    drop(memory);
    Ok(())
}
