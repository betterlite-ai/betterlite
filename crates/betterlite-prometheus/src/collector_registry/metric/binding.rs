use std::sync::{Arc, PoisonError};

use crate::collector_factory::collector::MetricUpdate;
use crate::multiprocess::writer::MultiProcessWriter;

use super::PrometheusMetric;

// multiple_inherent_impl leaves PrometheusMetric its one impl block, so binding is free here
pub(crate) fn bind_writer(
    metric: &PrometheusMetric,
    writer: &Arc<MultiProcessWriter>,
) -> Result<(), Arc<MultiProcessWriter>> {
    let mut slot = metric
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
    Ok(())
}

pub(crate) fn write_update(metric: &PrometheusMetric, update: MetricUpdate) {
    let slot = metric
        .multiprocess
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(writer) = slot {
        writer.update(&metric.description, &metric.label_values, update);
    }
}

// MetricWrapperBase.remove warns under multiprocess mode and leaves the entry to the readers
pub(crate) fn warn_removal(metric: &PrometheusMetric) {
    let bound = metric
        .multiprocess
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some();
    if bound {
        tracing::warn!("Removal of labels has not been implemented in  multi-process mode yet.");
    }
}
