use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use thiserror::Error;

use super::window::{BudgetWindowStore, WindowWrite};

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("the memory budget window store cannot be taken")]
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowObservation {
    pub started: bool,
    pub spend: Option<Decimal>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemoryEntry {
    pub start: Option<f64>,
    pub spend: Option<Decimal>,
    pub ttl: Option<Duration>,
}

#[derive(Debug, Default)]
pub struct MemoryStore {
    pub entries: Mutex<BTreeMap<String, MemoryEntry>>,
}

impl MemoryStore {
    pub fn seed_start(&self, key: &str, start: f64, ttl: Duration) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = entries.entry(key.to_owned()).or_default();
        entry.start = Some(start);
        entry.ttl = Some(ttl);
    }

    #[must_use]
    pub fn observe(&self, start_key: &str, spend_key: &str) -> WindowObservation {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let started = entries
            .get(start_key)
            .is_some_and(|entry| entry.start.is_some());
        let spend = entries.get(spend_key).and_then(|entry| entry.spend);
        WindowObservation { started, spend }
    }
}

#[async_trait]
impl BudgetWindowStore for MemoryStore {
    type Error = MemoryError;

    async fn read_start(&self, key: &str) -> Result<Option<f64>, Self::Error> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(entries.get(key).and_then(|entry| entry.start))
    }

    async fn has_spend(&self, key: &str) -> Result<bool, Self::Error> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(entries.get(key).is_some_and(|entry| entry.spend.is_some()))
    }

    async fn write(
        &self,
        key: &str,
        value: WindowWrite,
        ttl: Option<Duration>,
    ) -> Result<(), Self::Error> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = entries.entry(key.to_owned()).or_default();
        match value {
            WindowWrite::Start(start) => entry.start = Some(start),
            WindowWrite::Spend(spend) => entry.spend = Some(spend),
        }
        entry.ttl = ttl;
        Ok(())
    }
}
