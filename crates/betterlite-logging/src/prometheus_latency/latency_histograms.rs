use std::borrow::Cow;
use std::time::Instant;

use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use betterlite_prometheus::label_value_record::UserApiKeyLabelValues;
use rust_decimal::Decimal;

use crate::prometheus::bounded_series::CounterEmission;
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, RemovableMetric, SeriesLimits, TrackerError,
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
        labels: &ProjectedLabels,
        seconds: Decimal,
        now: Instant,
    ) -> Result<(), TrackerError<Metric::Error>> {
        self.handle.observe(labels, seconds);
        let emission = CounterEmission {
            counter: self.handle.metric,
            metric_name: self.metric_name,
            supported_labels: self.handle.supported_labels,
            tracker: self.tracker,
            limits: &self.limits,
        };
        emission.track_end_user(labels, now)
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
    record_time_to_first_token(histograms, observation, &latency_labels)?;
    record_api_latency(histograms, observation, end_time, &latency_labels)?;
    let queue_time = observation
        .queue_time_seconds
        .filter(|seconds| *seconds >= Decimal::ZERO);
    record_total_latency(
        histograms,
        observation,
        end_time,
        queue_time,
        &latency_labels,
    )?;
    record_queue_time(histograms, observation, queue_time, labels)?;
    Ok(())
}

fn record_time_to_first_token<Metric>(
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
    );
    match time_to_first_token {
        Some(seconds) if observation.stream == Some(true) => {
            let projected = prometheus_label_factory(
                histograms.time_to_first_token.handle.supported_labels,
                labels.values.as_ref(),
                labels.source(),
            )?;
            histograms.time_to_first_token.observe(
                &projected,
                seconds,
                observation.now_monotonic,
            )?;
        }
        Some(_) | None => tracing::debug!(
            "Time to first token metric not emitted, stream option in model_parameters is not True"
        ),
    }
    Ok(())
}

fn record_api_latency<Metric>(
    histograms: &LatencyHistograms<'_, Metric>,
    observation: &LatencyObservation<'_>,
    end_time: Decimal,
    labels: &LatencyLabels<'_>,
) -> Result<(), LatencyRecordingError<Metric::Error>>
where
    Metric: Collector + RemovableMetric,
{
    let Some(seconds) =
        safe_duration_seconds(observation.timestamps.api_call_start, Some(end_time))
    else {
        return Ok(());
    };
    let projected = prometheus_label_factory(
        histograms.api_latency.handle.supported_labels,
        labels.values.as_ref(),
        labels.source(),
    )?;
    histograms
        .api_latency
        .observe(&projected, seconds, observation.now_monotonic)?;
    Ok(())
}

fn record_total_latency<Metric>(
    histograms: &LatencyHistograms<'_, Metric>,
    observation: &LatencyObservation<'_>,
    end_time: Decimal,
    queue_time: Option<Decimal>,
    labels: &LatencyLabels<'_>,
) -> Result<(), LatencyRecordingError<Metric::Error>>
where
    Metric: Collector + RemovableMetric,
{
    let Some(total_seconds) =
        safe_duration_seconds(observation.timestamps.start_time, Some(end_time))
    else {
        return Ok(());
    };
    let observed_total = match queue_time {
        Some(added) => total_seconds
            .checked_add(added)
            .ok_or(LatencyRecordingError::Overflow)?,
        None => total_seconds,
    };
    let projected = prometheus_label_factory(
        histograms.total_latency.handle.supported_labels,
        labels.values.as_ref(),
        labels.source(),
    )?;
    histograms
        .total_latency
        .observe(&projected, observed_total, observation.now_monotonic)?;
    Ok(())
}

fn record_queue_time<Metric>(
    histograms: &LatencyHistograms<'_, Metric>,
    observation: &LatencyObservation<'_>,
    queue_time: Option<Decimal>,
    labels: &LabelInputs<'_>,
) -> Result<(), LatencyRecordingError<Metric::Error>>
where
    Metric: Collector + RemovableMetric,
{
    let Some(sample) = queue_time else {
        return Ok(());
    };
    let projected = prometheus_label_factory(
        histograms.queue_time.handle.supported_labels,
        labels.values,
        labels.source(),
    )?;
    histograms
        .queue_time
        .observe(&projected, sample, observation.now_monotonic)?;
    Ok(())
}

#[path = "tests/latency_histograms.rs"]
mod tests;
