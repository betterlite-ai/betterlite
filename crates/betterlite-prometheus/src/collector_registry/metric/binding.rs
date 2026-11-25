use std::sync::{Arc, PoisonError};

use crate::collector_factory::collector::MetricUpdate;
use crate::multiprocess::writer::{MultiProcessWriter, SeriesWrite};

use super::PrometheusMetric;
use super::gauge_mode::GaugeMode;

impl PrometheusMetric {
    // Gauge.__init__'s multiprocess_mode, which picks the gauge file this metric's values go into
    #[must_use]
    pub fn with_multiprocess_mode(mut self, mode: GaugeMode) -> Self {
        Arc::make_mut(&mut self.description).multiprocess_mode = mode;
        self
    }

    // the writer this metric writes through: one writer binds it, the same writer again is fine, another one is refused
    pub(crate) fn bind_writer(
        &self,
        writer: &Arc<MultiProcessWriter>,
    ) -> Result<(), Arc<MultiProcessWriter>> {
        let mut slot = self
            .multiprocess
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let bound = slot.clone();
        if let Some(existing) = bound {
            return if Arc::ptr_eq(&existing, writer) {
                Ok(())
            } else {
                Err(existing)
            };
        }
        *slot = Some(Arc::clone(writer));
        drop(slot);
        if self.description.label_names.is_empty() {
            writer.write_series(&self.description, &self.label_values, SeriesWrite::Create);
        }
        Ok(())
    }

    pub(crate) fn write_update(&self, update: MetricUpdate) {
        let slot = self
            .multiprocess
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(writer) = slot {
            writer.update(&self.description, &self.label_values, update);
        }
    }

    // MetricWrapperBase.remove warns under multiprocess mode and leaves the entry to the readers
    pub(crate) fn warn_removal(&self) {
        let bound = self
            .multiprocess
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if bound {
            tracing::warn!(
                "Removal of labels has not been implemented in  multi-process mode yet."
            );
        }
    }
}
