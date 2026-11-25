use std::sync::{Arc, PoisonError};
use std::time::SystemTime;

use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::collector_factory::collector::{Collector, MetricUpdate, MetricValue};

use super::sample::{Sample, added};
use super::{CollectorKind, MISSING_LABEL, PrometheusMetric};

impl Collector for PrometheusMetric {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let values: Vec<String> = self
            .description
            .label_names
            .iter()
            .map(|name| {
                labels
                    .iter()
                    .find(|(label, _)| *label == name.as_str())
                    .map_or_else(
                        || MISSING_LABEL.to_owned(),
                        |(_, value)| (*value).to_owned(),
                    )
            })
            .collect();
        Self {
            description: Arc::clone(&self.description),
            label_values: values,
            samples: Arc::clone(&self.samples),
            created: self.created,
            multiprocess: Arc::clone(&self.multiprocess),
        }
    }

    fn update(&self, update: MetricUpdate) {
        let mut samples = self.samples.lock().unwrap_or_else(PoisonError::into_inner);
        if update == MetricUpdate::Remove {
            let _removed = samples.shift_remove(&self.label_values);
            self.warn_removal();
            return;
        }
        let sample = self.sample_entry(&mut samples);
        let bounds = self.description.buckets.as_slice();
        match (self.description.kind, update) {
            (CollectorKind::Counter | CollectorKind::Gauge, MetricUpdate::Inc(value)) => {
                sample.total = added(sample.total, value);
            }
            (CollectorKind::Counter | CollectorKind::Gauge, MetricUpdate::Set(value)) => {
                sample.total = value;
            }
            (CollectorKind::Histogram, MetricUpdate::Observe(value)) => {
                sample.observe(value, bounds);
            }
            (CollectorKind::Counter | CollectorKind::Gauge, MetricUpdate::Observe(_))
            | (CollectorKind::Histogram, MetricUpdate::Inc(_) | MetricUpdate::Set(_))
            | (
                CollectorKind::Counter | CollectorKind::Gauge | CollectorKind::Histogram,
                MetricUpdate::Remove,
            ) => {}
        }
        // the bound writer takes the update before the samples lock lets the next one in, so both see the same order
        self.write_update(update);
        drop(samples);
    }
}

impl PrometheusMetric {
    // the series this handle owns: a label-less metric's is the constructor's, with the metric's own creation time; a child stamps now
    fn sample_entry<'samples>(
        &self,
        samples: &'samples mut IndexMap<Vec<String>, Sample>,
    ) -> &'samples mut Sample {
        let created = if self.description.label_names.is_empty() {
            self.created
        } else {
            SystemTime::now()
        };
        let bounds = self.description.buckets.as_slice();
        samples
            .entry(self.label_values.clone())
            .or_insert_with(|| Sample {
                total: MetricValue::Finite(Decimal::ZERO),
                count: 0,
                buckets: vec![0; bounds.len()],
                created,
            })
    }
}
