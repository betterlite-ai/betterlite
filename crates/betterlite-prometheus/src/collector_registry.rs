use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

pub mod metric;

use crate::multiprocess::writer::MultiProcessWriter;
use metric::{CollectorDescription, PrometheusMetric};

mod tests;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CollectorRegistryError {
    #[error("Duplicated timeseries in CollectorRegistry: {}", names.join(", "))]
    DuplicatedTimeseries { names: Vec<String> },
    #[error(
        "the metric {name} is already bound to the multiprocess writer in {}",
        path.display()
    )]
    MultiprocessBound { name: String, path: PathBuf },
}

#[derive(Debug, Default)]
pub struct CollectorRegistry {
    multiprocess: Option<Arc<MultiProcessWriter>>,
    state: Mutex<RegistryState>,
}

#[derive(Debug, Default)]
struct RegistryState {
    samples: BTreeMap<String, PrometheusMetric>,
    collectors: Vec<PrometheusMetric>,
}

impl CollectorRegistry {
    // the one registry a worker builds when PROMETHEUS_MULTIPROC_DIR is set
    #[must_use]
    pub fn multiprocess(writer: MultiProcessWriter) -> Self {
        Self {
            multiprocess: Some(Arc::new(writer)),
            state: Mutex::new(RegistryState::default()),
        }
    }

    #[must_use]
    pub fn multiprocess_dir(&self) -> Option<&Path> {
        self.multiprocess
            .as_deref()
            .map(|writer| writer.directory.as_path())
    }

    // CollectorRegistry.register: every sample name a collector owns is claimed before any of them is stored
    pub fn register(&self, collector: PrometheusMetric) -> Result<(), CollectorRegistryError> {
        let names = collector.sample_names();
        {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let duplicates: Vec<String> = names
                .iter()
                .filter(|name| state.samples.contains_key(*name))
                .map(|name| (*name).clone())
                .collect();
            if !duplicates.is_empty() {
                return Err(CollectorRegistryError::DuplicatedTimeseries { names: duplicates });
            }
        }
        // the writer's lock is taken without the registry lock, which the duplicate check released at once
        if let Some(writer) = &self.multiprocess {
            collector.bind_writer(writer).map_err(|bound| {
                CollectorRegistryError::MultiprocessBound {
                    name: collector.description.name.clone(),
                    path: bound.directory.clone(),
                }
            })?;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        for name in names {
            let _replaced = state.samples.insert(name, collector.clone());
        }
        state.collectors.push(collector);
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
