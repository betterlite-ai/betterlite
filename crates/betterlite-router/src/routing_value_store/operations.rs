use std::collections::BTreeSet;
use std::slice;
use std::sync::{Mutex, PoisonError};

use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use rust_decimal::Decimal;

use super::codec::{from_cache_value, to_cache_value};
use super::{
    Clock, CompareOutcome, RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreError,
    StoreRead, StoreSettings,
};

impl RoutingValueStore {
    pub fn new(settings: StoreSettings, clock: Clock) -> Result<Self, StoreError> {
        let cache = LocalTtlCache::with_clock(
            settings.max_size_in_memory,
            settings.default_ttl_seconds,
            settings.max_size_per_item,
            settings.loaded_max_size_per_item_kib,
            clock,
        )?;
        Ok(Self {
            cache: Mutex::new(cache),
        })
    }

    pub fn set(
        &self,
        key: &RouteKey,
        value: &RouteValue,
        ttl_seconds: Option<f64>,
    ) -> Result<(), StoreError> {
        let Some(encoded) = to_cache_value(value)? else {
            return Ok(());
        };
        self.store(key, encoded, ttl_seconds)
    }

    fn store(
        &self,
        key: &RouteKey,
        encoded: CacheValue,
        ttl_seconds: Option<f64>,
    ) -> Result<(), StoreError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.set(key.as_ref(), encoded, ttl_seconds)?;
        drop(cache);
        Ok(())
    }

    pub fn get(&self, key: &RouteKey) -> Result<StoreRead, StoreError> {
        self.batch_get(slice::from_ref(key))
            .map(|mut values| values.pop().unwrap_or(StoreRead::Missing))
    }

    pub fn batch_get(&self, keys: &[RouteKey]) -> Result<Vec<StoreRead>, StoreError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        keys.iter()
            .map(|key| read(&mut cache, key))
            .collect::<Result<Vec<StoreRead>, StoreError>>()
    }

    pub fn set_pipeline(
        &self,
        entries: &[(RouteKey, RouteValue)],
        ttl_seconds: Option<f64>,
    ) -> Result<(), StoreError> {
        for (key, value) in entries {
            self.set(key, value, ttl_seconds)?;
        }
        Ok(())
    }

    pub fn set_add(
        &self,
        key: &RouteKey,
        values: &[String],
        ttl_seconds: Option<f64>,
    ) -> Result<Vec<String>, StoreError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let mut members = members(&mut cache, key)?;
        for value in values {
            members.insert(value.clone());
        }
        if let Some(encoded) = to_cache_value(&RouteValue::Members(members))? {
            cache.set(key.as_ref(), encoded, ttl_seconds)?;
        }
        drop(cache);
        Ok(values.to_vec())
    }

    pub fn increment(
        &self,
        key: &RouteKey,
        by: &RouteIncrement,
        ttl_seconds: Option<f64>,
    ) -> Result<RouteValue, StoreError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let next = match (read(&mut cache, key)?, by) {
            (StoreRead::Missing, RouteIncrement::Counter(increment)) => {
                RouteValue::Counter(*increment)
            }
            (StoreRead::Missing, RouteIncrement::Spend(increment)) => RouteValue::Spend(*increment),
            (
                StoreRead::Value(RouteValue::Counter(current)),
                RouteIncrement::Counter(increment),
            ) => {
                let sum =
                    current
                        .checked_add(*increment)
                        .ok_or_else(|| StoreError::CounterOverflow {
                            key: key.as_ref().to_owned(),
                        })?;
                RouteValue::Counter(sum)
            }
            (StoreRead::Value(RouteValue::Spend(current)), RouteIncrement::Spend(increment)) => {
                let sum =
                    current
                        .checked_add(*increment)
                        .ok_or_else(|| StoreError::SpendOverflow {
                            key: key.as_ref().to_owned(),
                        })?;
                RouteValue::Spend(sum)
            }
            (StoreRead::Value(RouteValue::Counter(current)), RouteIncrement::Spend(increment)) => {
                let sum = Decimal::from(current)
                    .checked_add(*increment)
                    .ok_or_else(|| StoreError::SpendOverflow {
                        key: key.as_ref().to_owned(),
                    })?;
                RouteValue::Spend(sum)
            }
            (StoreRead::Value(RouteValue::Spend(current)), RouteIncrement::Counter(increment)) => {
                let sum = current
                    .checked_add(Decimal::from(*increment))
                    .ok_or_else(|| StoreError::SpendOverflow {
                        key: key.as_ref().to_owned(),
                    })?;
                RouteValue::Spend(sum)
            }
            (StoreRead::Value(_), RouteIncrement::Counter(_) | RouteIncrement::Spend(_)) => {
                return Err(StoreError::NotIncrementable {
                    key: key.as_ref().to_owned(),
                });
            }
        };
        let Some(encoded) = to_cache_value(&next)? else {
            return Ok(next);
        };
        cache.set(key.as_ref(), encoded, ttl_seconds)?;
        drop(cache);
        Ok(next)
    }

    pub fn compare_and_set(
        &self,
        key: &RouteKey,
        expected: &StoreRead,
        value: &RouteValue,
        ttl_seconds: Option<f64>,
    ) -> Result<CompareOutcome, StoreError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let current = read(&mut cache, key)?;
        if current != *expected {
            return Ok(CompareOutcome {
                swapped: false,
                current,
            });
        }
        let Some(encoded) = to_cache_value(value)? else {
            return Ok(CompareOutcome {
                swapped: false,
                current,
            });
        };
        cache.set(key.as_ref(), encoded, ttl_seconds)?;
        drop(cache);
        Ok(CompareOutcome {
            swapped: true,
            current: StoreRead::Value(value.clone()),
        })
    }

    pub fn delete(&self, key: &RouteKey) {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .delete(key.as_ref());
    }
}

fn read(cache: &mut LocalTtlCache, key: &RouteKey) -> Result<StoreRead, StoreError> {
    match cache.get(key.as_ref())? {
        None => Ok(StoreRead::Missing),
        Some(value) => Ok(StoreRead::Value(from_cache_value(&value)?)),
    }
}

fn members(cache: &mut LocalTtlCache, key: &RouteKey) -> Result<BTreeSet<String>, StoreError> {
    match read(cache, key)? {
        StoreRead::Missing => Ok(BTreeSet::new()),
        StoreRead::Value(RouteValue::Members(members)) => Ok(members),
        StoreRead::Value(_) => Err(StoreError::NotMembers {
            key: key.as_ref().to_owned(),
        }),
    }
}
