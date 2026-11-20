use std::borrow::Cow;
use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use crate::prometheus::bounded_series::CounterEmission;
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, RemovableMetric, SeriesLimits,
};
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_projection::prometheus_label_factory::{
    LabelSource, ProjectedLabels, prometheus_label_factory,
};
use crate::prometheus_label_projection::tag_wildcard_matching::TagPatternError;

use super::error::LatencyRecordingError;
use super::labels::LabelInputs;

// prometheus_client renders every label value with str(), and Python renders None as "None"
const MISSING_LABEL_VALUE: &str = "None";

#[derive(Debug)]
pub struct MetricHandle<'labels, Metric> {
    pub metric: &'labels Metric,
    pub supported_labels: &'labels [String],
}

impl<Metric: Collector> MetricHandle<'_, Metric> {
    pub fn observe(&self, labels: &ProjectedLabels, seconds: Decimal) {
        let pairs: Vec<(&str, &str)> = labels
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str(),
                    value.as_deref().unwrap_or(MISSING_LABEL_VALUE),
                )
            })
            .collect();
        self.metric
            .labels(&pairs)
            .observe(MetricValue::Finite(seconds));
    }
}

#[derive(Debug)]
pub struct LatencyHistogram<'labels, Metric> {
    pub handle: MetricHandle<'labels, Metric>,
    pub metric_name: &'static str,
    pub tracker: &'labels BoundedPrometheusSeriesTracker,
    pub limits: SeriesLimits,
}

impl<Metric> LatencyHistogram<'_, Metric>
where
    Metric: Collector + RemovableMetric,
{
    pub fn observe(
        &self,
        values: &UserApiKeyLabelValues,
        source: LabelSource<'_>,
        seconds: Decimal,
        now: Instant,
    ) -> Result<(), LatencyRecordingError<Metric::Error>> {
        let labels = prometheus_label_factory(self.handle.supported_labels, values, source)?;
        self.handle.observe(&labels, seconds);
        let emission = CounterEmission {
            counter: self.handle.metric,
            metric_name: self.metric_name,
            supported_labels: self.handle.supported_labels,
            tracker: self.tracker,
            limits: &self.limits,
        };
        emission.track_end_user(&labels, now)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct LatencyHistograms<'labels, Metric> {
    pub time_to_first_token: LatencyHistogram<'labels, Metric>,
    pub api_latency: LatencyHistogram<'labels, Metric>,
    pub total_latency: LatencyHistogram<'labels, Metric>,
    pub queue_time: LatencyHistogram<'labels, Metric>,
}

#[derive(Clone, Copy, Debug)]
pub struct LatencyTimestamps {
    pub start_time: Option<Decimal>,
    pub end_time: Option<Decimal>,
    pub api_call_start: Option<Decimal>,
    pub completion_start: Option<Decimal>,
}

#[derive(Clone, Copy, Debug)]
pub struct LatencyObservation<'observation> {
    pub timestamps: LatencyTimestamps,
    pub now: Decimal,
    pub now_monotonic: Instant,
    pub stream: Option<bool>,
    pub queue_time_seconds: Option<Decimal>,
    pub input_sequence_length: Option<&'observation str>,
}

// the first three latency histograms carry a copy of the values with input_sequence_length set and its own context, so the end-user cache is not shared
struct LatencyLabels<'labels> {
    values: Cow<'labels, UserApiKeyLabelValues>,
    context: Option<PrometheusLabelFactoryContext>,
    original: LabelInputs<'labels>,
}

impl<'labels> LatencyLabels<'labels> {
    fn new(
        labels: LabelInputs<'labels>,
        input_sequence_length: Option<&str>,
    ) -> Result<Self, TagPatternError> {
        let Some(length) = input_sequence_length else {
            return Ok(Self {
                values: Cow::Borrowed(labels.values),
                context: None,
                original: labels,
            });
        };
        let mut values = labels.values.clone();
        values.input_sequence_length = Some(length.to_owned());
        let context =
            PrometheusLabelFactoryContext::new(&values, labels.settings, labels.end_user_settings)?;
        Ok(Self {
            values: Cow::Owned(values),
            context: Some(context),
            original: labels,
        })
    }

    fn values(&self) -> &UserApiKeyLabelValues {
        self.values.as_ref()
    }

    const fn source(&self) -> LabelSource<'_> {
        match &self.context {
            Some(context) => LabelSource::Context(context),
            None => self.original.source(),
        }
    }
}

#[must_use]
pub fn safe_duration_seconds(
    start_time: Option<Decimal>,
    end_time: Option<Decimal>,
) -> Option<Decimal> {
    let start = start_time?;
    let end = end_time?;
    end.checked_sub(start)
}

pub fn set_latency_metrics<Metric>(
    histograms: &LatencyHistograms<'_, Metric>,
    observation: &LatencyObservation<'_>,
    labels: &LabelInputs<'_>,
) -> Result<(), LatencyRecordingError<Metric::Error>>
where
    Metric: Collector + RemovableMetric,
{
    let end_time = observation.timestamps.end_time.unwrap_or(observation.now);
    let latency_labels = LatencyLabels::new(*labels, observation.input_sequence_length)?;
    observe_time_to_first_token(histograms, observation, &latency_labels)?;
    if let Some(seconds) =
        safe_duration_seconds(observation.timestamps.api_call_start, Some(end_time))
    {
        histograms.api_latency.observe(
            latency_labels.values(),
            latency_labels.source(),
            seconds,
            observation.now_monotonic,
        )?;
    }
    observe_total_and_queue_latency(histograms, observation, end_time, labels, &latency_labels)
}

fn observe_time_to_first_token<Metric>(
    histograms: &LatencyHistograms<'_, Metric>,
    observation: &LatencyObservation<'_>,
    labels: &LatencyLabels<'_>,
) -> Result<(), LatencyRecordingError<Metric::Error>>
where
    Metric: Collector + RemovableMetric,
{
    let time_to_first_token = safe_duration_seconds(
        observation.timestamps.api_call_start,
        observation.timestamps.completion_start,
    )
    .filter(|_| observation.stream == Some(true));
    let Some(seconds) = time_to_first_token else {
        tracing::debug!(
            "Time to first token metric not emitted, stream option in model_parameters is not True"
        );
        return Ok(());
    };
    histograms.time_to_first_token.observe(
        labels.values(),
        labels.source(),
        seconds,
        observation.now_monotonic,
    )
}

fn observe_total_and_queue_latency<Metric>(
    histograms: &LatencyHistograms<'_, Metric>,
    observation: &LatencyObservation<'_>,
    end_time: Decimal,
    labels: &LabelInputs<'_>,
    latency_labels: &LatencyLabels<'_>,
) -> Result<(), LatencyRecordingError<Metric::Error>>
where
    Metric: Collector + RemovableMetric,
{
    let queue_time = observation
        .queue_time_seconds
        .filter(|seconds| *seconds >= Decimal::ZERO);
    if let Some(total_seconds) =
        safe_duration_seconds(observation.timestamps.start_time, Some(end_time))
    {
        let observed_total = match queue_time {
            Some(added) => total_seconds
                .checked_add(added)
                .ok_or(LatencyRecordingError::Overflow)?,
            None => total_seconds,
        };
        histograms.total_latency.observe(
            latency_labels.values(),
            latency_labels.source(),
            observed_total,
            observation.now_monotonic,
        )?;
    }
    if let Some(sample) = queue_time {
        histograms.queue_time.observe(
            labels.values,
            labels.source(),
            sample,
            observation.now_monotonic,
        )?;
    }
    Ok(())
}

#[path = "tests/latency_histograms.rs"]
mod tests;
