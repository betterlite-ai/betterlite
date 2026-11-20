use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use indexmap::IndexMap;

// LiteLLM's _RemovableMetric protocol: implementors map metric.remove() raising KeyError to Absent, AttributeError or ValueError to Unsupported, and anything else to Err
pub trait RemovableMetric {
    type Error: Error + 'static;

    fn remove_child(&self, label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildRemoval {
    Removed,
    Absent,
    Unsupported,
}

#[derive(Debug, thiserror::Error)]
pub enum TrackerError<MetricError: Error + 'static> {
    #[error("the metric failed to remove one of its series")]
    Removal {
        #[source]
        source: MetricError,
    },
}

// the metric name and label tuple that name one tracked series
#[derive(Clone, Copy, Debug)]
pub struct SeriesKey<'key> {
    pub metric_name: &'key str,
    pub label_values: &'key [Option<String>],
}

// max_series at or below zero is unlimited, and an absent or zero cleanup interval runs the ttl pass on every call
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SeriesLimits {
    pub max_series: Option<i64>,
    pub ttl: Option<Duration>,
    pub cleanup_interval: Option<Duration>,
}

type TrackedSeries = IndexMap<Vec<Option<String>>, Instant>;

#[derive(Debug, Default)]
struct TrackerState {
    series: BTreeMap<String, TrackedSeries>,
    last_ttl_cleanup: BTreeMap<String, Instant>,
}

#[derive(Debug, Default)]
pub struct BoundedPrometheusSeriesTracker {
    state: Mutex<TrackerState>,
}

impl BoundedPrometheusSeriesTracker {
    pub fn track_series<M: RemovableMetric>(
        &self,
        metric: &M,
        series: SeriesKey<'_>,
        limits: SeriesLimits,
        now: Instant,
    ) -> Result<(), TrackerError<M::Error>> {
        if limits.max_series.is_none() && limits.ttl.is_none() {
            return Ok(());
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let TrackerState {
            series: tracked,
            last_ttl_cleanup,
        } = &mut *state;
        let entries = tracked.entry(series.metric_name.to_owned()).or_default();
        record_series(entries, series.label_values, now);
        if let Some(ttl) = limits.ttl
            && should_run_ttl_cleanup(
                last_ttl_cleanup,
                series.metric_name,
                now,
                limits.cleanup_interval,
            )
        {
            let expired = expired_series(entries, now, ttl);
            remove_expired(self, metric, entries, &expired)?;
        }
        evict_series(self, entries, limits.max_series, metric)
    }

    pub fn remove_series<M: RemovableMetric>(
        &self,
        metric: &M,
        label_values: &[Option<String>],
    ) -> Result<bool, TrackerError<M::Error>> {
        match metric.remove_child(label_values) {
            Ok(ChildRemoval::Removed | ChildRemoval::Absent) => Ok(true),
            Ok(ChildRemoval::Unsupported) => Ok(false),
            Err(source) => Err(TrackerError::Removal { source }),
        }
    }
}

// recording the newest sample first keeps an evicted series from dropping the sample that pushed it over the limit
fn record_series(entries: &mut TrackedSeries, label_values: &[Option<String>], now: Instant) {
    let (index, _previous) = entries.insert_full(label_values.to_vec(), now);
    let last = entries.len().saturating_sub(1);
    entries.move_index(index, last);
}

fn expired_series(
    entries: &TrackedSeries,
    now: Instant,
    ttl: Duration,
) -> Vec<Vec<Option<String>>> {
    entries
        .iter()
        .filter(|(_, last_seen)| now.saturating_duration_since(**last_seen) > ttl)
        .map(|(label_values, _)| label_values.clone())
        .collect()
}

fn remove_expired<M: RemovableMetric>(
    tracker: &BoundedPrometheusSeriesTracker,
    metric: &M,
    entries: &mut TrackedSeries,
    expired: &[Vec<Option<String>>],
) -> Result<(), TrackerError<M::Error>> {
    for label_values in expired {
        if tracker.remove_series(metric, label_values)? {
            entries.shift_remove(label_values);
        }
    }
    Ok(())
}

// max_series at or below zero is unlimited, and a refused removal keeps the series so a later call retries it
fn evict_series<M: RemovableMetric>(
    tracker: &BoundedPrometheusSeriesTracker,
    entries: &mut TrackedSeries,
    max_series: Option<i64>,
    metric: &M,
) -> Result<(), TrackerError<M::Error>> {
    let Some(max_series) = max_series.filter(|max| *max > 0) else {
        return Ok(());
    };
    let limit = usize::try_from(max_series).ok();
    while limit.is_some_and(|ceiling| entries.len() > ceiling) {
        let Some(oldest) = entries.keys().next().cloned() else {
            break;
        };
        if !tracker.remove_series(metric, &oldest)? {
            break;
        }
        entries.shift_remove(&oldest);
    }
    Ok(())
}

// an absent or zero interval runs the pass on every call, so every elapsed time clears it
fn should_run_ttl_cleanup(
    last_ttl_cleanup: &mut BTreeMap<String, Instant>,
    metric_name: &str,
    now: Instant,
    cleanup_interval: Option<Duration>,
) -> bool {
    let due = cleanup_interval.is_none_or(|interval| {
        last_ttl_cleanup
            .get(metric_name)
            .is_none_or(|last_cleanup| now.saturating_duration_since(*last_cleanup) >= interval)
    });
    if due {
        last_ttl_cleanup.insert(metric_name.to_owned(), now);
    }
    due
}

mod tests;
