use std::error::Error as StdError;
use std::time::Instant;

use betterlite_config::sections::prometheus_bounded_series::Settings as BoundedSeriesSettings;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use betterlite_prometheus::label_names_catalog::UserApiKeyLabelName;
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;

use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, RemovableMetric, SeriesKey, SeriesLimits, TrackerError,
};
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelFactoryError, LabelSource, ProjectedLabels, prometheus_label_factory,
};

const END_USER_LABEL: &str = UserApiKeyLabelName::EndUser.as_str();
// prometheus_client renders every label value with str(), and Python renders None as "None"
const MISSING_LABEL_VALUE: &str = "None";

// one counter and the tracker state that caps the series of the metrics carrying the end_user label
#[derive(Debug)]
pub struct CounterEmission<'emission, Counter: Collector + RemovableMetric> {
    pub counter: &'emission Counter,
    pub metric_name: &'emission str,
    pub supported_labels: &'emission [String],
    pub tracker: &'emission BoundedPrometheusSeriesTracker,
    pub limits: &'emission SeriesLimits,
}

#[derive(Debug, thiserror::Error)]
pub enum CounterEmissionError<MetricError>
where
    MetricError: StdError + 'static,
{
    #[error(transparent)]
    Labels(#[from] LabelFactoryError),
    #[error(transparent)]
    Series(#[from] TrackerError<MetricError>),
}

impl<Counter: Collector + RemovableMetric> CounterEmission<'_, Counter> {
    pub fn inc_labeled_counter(
        &self,
        values: &UserApiKeyLabelValues,
        source: LabelSource<'_>,
        amount: MetricValue,
        now: Instant,
    ) -> Result<(), CounterEmissionError<Counter::Error>> {
        let labels = prometheus_label_factory(self.supported_labels, values, source)?;
        self.increment(&labels, amount);
        self.track_end_user(&labels, now)?;
        Ok(())
    }

    pub fn track_end_user(
        &self,
        labels: &ProjectedLabels,
        now: Instant,
    ) -> Result<(), TrackerError<Counter::Error>> {
        if !self
            .supported_labels
            .iter()
            .any(|label| label == END_USER_LABEL)
        {
            return Ok(());
        }
        if labels.get(END_USER_LABEL).is_none_or(Option::is_none) {
            return Ok(());
        }
        if self.limits.max_series.is_none() && self.limits.ttl.is_none() {
            return Ok(());
        }
        let label_values: Vec<Option<String>> = self
            .supported_labels
            .iter()
            .map(|label| labels.get(label).cloned().flatten())
            .collect();
        self.tracker.track_series(
            self.counter,
            SeriesKey {
                metric_name: self.metric_name,
                label_values: &label_values,
            },
            *self.limits,
            now,
        )
    }

    fn increment(&self, labels: &ProjectedLabels, amount: MetricValue) {
        let pairs: Vec<(&str, &str)> = labels
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str(),
                    value.as_deref().unwrap_or(MISSING_LABEL_VALUE),
                )
            })
            .collect();
        self.counter.labels(&pairs).inc(amount);
    }
}

impl From<&BoundedSeriesSettings> for SeriesLimits {
    fn from(settings: &BoundedSeriesSettings) -> Self {
        Self {
            max_series: settings.max_series,
            ttl: settings.ttl,
            cleanup_interval: settings.cleanup_interval,
        }
    }
}

mod tests;
