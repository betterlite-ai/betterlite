mod tests;

use std::sync::{Mutex, MutexGuard, PoisonError};

use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::collector_registry::metric::{
    Bucket, CollectorKind, GaugeMode, PrometheusMetric,
};
use betterlite_prometheus::collector_registry::{CollectorRegistry, CollectorRegistryError};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

const ROWS_DELETED_NAME: &str = "litellm_spend_log_cleanup_rows_deleted_total";
const BATCH_DURATION_NAME: &str = "litellm_spend_log_cleanup_batch_duration_seconds";
const ROWS_REMAINING_NAME: &str = "litellm_spend_log_cleanup_rows_remaining";
const BATCH_FAILURES_NAME: &str = "litellm_spend_log_cleanup_batch_failures_total";
const RUNS_NAME: &str = "litellm_spend_log_cleanup_runs_total";

const ROWS_DELETED_HELP: &str = "Rows deleted by the spend-log retention cleanup job";
const BATCH_DURATION_HELP: &str = "Wall-clock duration of one retention cleanup delete batch";
const ROWS_REMAINING_HELP: &str = "Expired rows still awaiting deletion, counted only up to SPEND_LOG_CLEANUP_REMAINING_COUNT_CAP so the probe itself cannot scan a large table; a value equal to that cap means at least that many remain";
const BATCH_FAILURES_HELP: &str = "Retention cleanup delete batches that raised";
const RUNS_HELP: &str = "Retention cleanup runs, labelled by why the run ended";

const TABLE_LABEL: &str = "table";
const OUTCOME_LABEL: &str = "outcome";

// _BATCH_DURATION_BUCKETS with the +Inf bound Histogram._prepare_buckets appends
const BATCH_DURATION_BUCKETS: [Bucket; 12] = [
    Bucket::Seconds(0.005),
    Bucket::Seconds(0.025),
    Bucket::Seconds(0.1),
    Bucket::Seconds(0.25),
    Bucket::Seconds(0.5),
    Bucket::Seconds(1.0),
    Bucket::Seconds(2.5),
    Bucket::Seconds(5.0),
    Bucket::Seconds(10.0),
    Bucket::Seconds(30.0),
    Bucket::Seconds(60.0),
    Bucket::Infinite,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    BudgetExhausted,
    BatchCapReached,
    SkippedLocked,
    SkippedDisabled,
    Aborted,
}

impl RunOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::BudgetExhausted => "budget_exhausted",
            Self::BatchCapReached => "batch_cap_reached",
            Self::SkippedLocked => "skipped_locked",
            Self::SkippedDisabled => "skipped_disabled",
            Self::Aborted => "aborted",
        }
    }
}

#[derive(Debug, Default)]
struct Instruments {
    rows_deleted: Option<PrometheusMetric>,
    batch_duration: Option<PrometheusMetric>,
    rows_remaining: Option<PrometheusMetric>,
    batch_failures: Option<PrometheusMetric>,
    runs: Option<PrometheusMetric>,
}

#[derive(Debug, Default)]
struct Registration {
    attempted: bool,
    instruments: Instruments,
}

struct Instrument {
    kind: CollectorKind,
    name: &'static str,
    help: &'static str,
    label: &'static str,
    buckets: &'static [Bucket],
    mode: GaugeMode,
}

const ROWS_DELETED: Instrument = Instrument {
    kind: CollectorKind::Counter,
    name: ROWS_DELETED_NAME,
    help: ROWS_DELETED_HELP,
    label: TABLE_LABEL,
    buckets: &[],
    mode: GaugeMode::All,
};

const BATCH_DURATION: Instrument = Instrument {
    kind: CollectorKind::Histogram,
    name: BATCH_DURATION_NAME,
    help: BATCH_DURATION_HELP,
    label: TABLE_LABEL,
    buckets: &BATCH_DURATION_BUCKETS,
    mode: GaugeMode::All,
};

// spend_log_cleanup_metrics.py's rows remaining gauge is a Gauge in livemax mode
const ROWS_REMAINING: Instrument = Instrument {
    kind: CollectorKind::Gauge,
    name: ROWS_REMAINING_NAME,
    help: ROWS_REMAINING_HELP,
    label: TABLE_LABEL,
    buckets: &[],
    mode: GaugeMode::LiveMax,
};

const BATCH_FAILURES: Instrument = Instrument {
    kind: CollectorKind::Counter,
    name: BATCH_FAILURES_NAME,
    help: BATCH_FAILURES_HELP,
    label: TABLE_LABEL,
    buckets: &[],
    mode: GaugeMode::All,
};

const RUNS: Instrument = Instrument {
    kind: CollectorKind::Counter,
    name: RUNS_NAME,
    help: RUNS_HELP,
    label: OUTCOME_LABEL,
    buckets: &[],
    mode: GaugeMode::All,
};

pub struct CleanupMetrics<'registry> {
    registry: &'registry CollectorRegistry,
    registration: Mutex<Registration>,
}

impl<'registry> CleanupMetrics<'registry> {
    #[must_use]
    pub const fn new(registry: &'registry CollectorRegistry) -> Self {
        Self {
            registry,
            registration: Mutex::new(Registration {
                attempted: false,
                instruments: Instruments {
                    rows_deleted: None,
                    batch_duration: None,
                    rows_remaining: None,
                    batch_failures: None,
                    runs: None,
                },
            }),
        }
    }

    // _ensure_initialized: one attempt, and an instrument that failed takes none of the later ones with it
    fn ensure_registered(&self) {
        let mut registration = self
            .registration
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if registration.attempted {
            return;
        }
        registration.attempted = true;
        match register_instruments(self.registry, &mut registration.instruments) {
            Ok(()) => {}
            Err(error) => tracing::warn!("Could not register spend-log cleanup metrics: {error}"),
        }
    }

    fn locked_registration(&self) -> MutexGuard<'_, Registration> {
        self.ensure_registered();
        self.registration
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn record_batch(&self, table_name: &str, rows_deleted: u64, duration_seconds: f64) {
        let registration = self.locked_registration();
        let labels = [(TABLE_LABEL, table_name)];
        if let Some(metric) = &registration.instruments.rows_deleted {
            metric
                .labels(&labels)
                .inc(MetricValue::Finite(Decimal::from(rows_deleted)));
        }
        if let Some(metric) = &registration.instruments.batch_duration {
            metric
                .labels(&labels)
                .observe(duration_value(duration_seconds));
        }
    }

    pub fn record_batch_failure(&self, table_name: &str) {
        let registration = self.locked_registration();
        if let Some(metric) = &registration.instruments.batch_failures {
            metric
                .labels(&[(TABLE_LABEL, table_name)])
                .inc(MetricValue::Finite(Decimal::ONE));
        }
    }

    pub fn set_rows_remaining(&self, table_name: &str, remaining: i64) {
        let registration = self.locked_registration();
        if let Some(metric) = &registration.instruments.rows_remaining {
            metric
                .labels(&[(TABLE_LABEL, table_name)])
                .set(MetricValue::Finite(Decimal::from(remaining)));
        }
    }

    pub fn record_run(&self, outcome: RunOutcome) {
        let registration = self.locked_registration();
        if let Some(metric) = &registration.instruments.runs {
            metric
                .labels(&[(OUTCOME_LABEL, outcome.as_str())])
                .inc(MetricValue::Finite(Decimal::ONE));
        }
    }
}

fn register_instruments(
    registry: &CollectorRegistry,
    instruments: &mut Instruments,
) -> Result<(), CollectorRegistryError> {
    instruments.rows_deleted = Some(install(registry, &ROWS_DELETED)?);
    instruments.batch_duration = Some(install(registry, &BATCH_DURATION)?);
    instruments.rows_remaining = Some(install(registry, &ROWS_REMAINING)?);
    instruments.batch_failures = Some(install(registry, &BATCH_FAILURES)?);
    instruments.runs = Some(install(registry, &RUNS)?);
    Ok(())
}

fn install(
    registry: &CollectorRegistry,
    instrument: &Instrument,
) -> Result<PrometheusMetric, CollectorRegistryError> {
    let metric = PrometheusMetric::new(
        instrument.kind,
        instrument.name,
        instrument.help,
        &[instrument.label],
        instrument.buckets.to_vec(),
    )
    .with_multiprocess_mode(instrument.mode);
    registry.register(metric.clone())?;
    Ok(metric)
}

fn duration_value(seconds: f64) -> MetricValue {
    if seconds.is_nan() {
        return MetricValue::NotANumber;
    }
    match Decimal::from_f64(seconds) {
        Some(value) => MetricValue::Finite(value),
        None if seconds.is_sign_negative() => MetricValue::NegativeInfinity,
        None => MetricValue::PositiveInfinity,
    }
}
