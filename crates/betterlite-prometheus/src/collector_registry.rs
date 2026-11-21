use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

pub mod metric;

use metric::{CollectorDescription, PrometheusMetric};

mod tests;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CollectorRegistryError {
    #[error("Duplicated timeseries in CollectorRegistry: {}", names.join(", "))]
    DuplicatedTimeseries { names: Vec<String> },
}

#[derive(Debug, Default)]
pub struct CollectorRegistry {
    state: Mutex<RegistryState>,
}

#[derive(Debug, Default)]
struct RegistryState {
    samples: BTreeMap<String, PrometheusMetric>,
    collectors: Vec<PrometheusMetric>,
}

impl CollectorRegistry {
    // CollectorRegistry.register: every sample name a collector owns is claimed before any of them is stored
    pub fn register(&self, collector: PrometheusMetric) -> Result<(), CollectorRegistryError> {
        let names = collector.sample_names();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let duplicates: Vec<String> = names
            .iter()
            .filter(|name| state.samples.contains_key(*name))
            .map(|name| (*name).clone())
            .collect();
        if !duplicates.is_empty() {
            return Err(CollectorRegistryError::DuplicatedTimeseries { names: duplicates });
        }
        for name in names {
            state.samples.insert(name, collector.clone());
        }
        state.collectors.push(collector);
        drop(state);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<PrometheusMetric> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .samples
            .get(name)
            .cloned()
    }

    #[must_use]
    pub fn descriptions(&self) -> Vec<CollectorDescription> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .collectors
            .iter()
            .map(|collector| CollectorDescription::clone(&collector.description))
            .collect()
    }

    #[must_use]
    pub fn metrics(&self) -> Vec<PrometheusMetric> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .collectors
            .clone()
    }
}
