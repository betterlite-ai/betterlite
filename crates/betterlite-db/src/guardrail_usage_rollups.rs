use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::spend_batches::StatementLimits;
use crate::tool_spend::RetryPause;
use payload::{GuardrailEntry, PayloadMetadata, PayloadStartTime};
use pending::PendingRollups;

pub mod flush;
pub mod index_rows;
pub mod payload;
pub mod pending;
pub mod rollups;

mod tests;

// LiteLLM retries an upsert only for DB_RETRY_SAFE_ERROR_TYPES (httpx.ConnectError) and sleeps 2 ** retries_left seconds between attempts
pub const UPSERT_RETRY_TIMES: u32 = 3;

// LiteLLM caps each pending queue on its own: daily metrics and usage units never share the limit
pub const MAX_PENDING_ROWS: usize = 10_000;

const METRICS_LABEL: &str = "daily metrics";
const UNITS_LABEL: &str = "usage unit";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardrailAction {
    Passed,
    Blocked,
    Flagged,
    NotRun,
}

#[must_use]
pub fn guardrail_status_to_action(status: Option<&str>) -> GuardrailAction {
    let Some(status) = status else {
        return GuardrailAction::Passed;
    };
    let lowered = status.to_lowercase();
    if lowered == "not_run" {
        return GuardrailAction::NotRun;
    }
    if lowered.contains("intervened") || lowered.contains("block") {
        return GuardrailAction::Blocked;
    }
    if lowered.contains("flagged") || lowered.contains("fail") || lowered.contains("error") {
        return GuardrailAction::Flagged;
    }
    GuardrailAction::Passed
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct GuardrailSpendLog {
    pub request_id: Option<String>,
    #[serde(rename = "startTime")]
    pub start_time: Option<PayloadStartTime>,
    pub team_id: Option<String>,
    pub api_key: Option<String>,
    pub metadata: Option<PayloadMetadata>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GuardrailMetadata {
    pub guardrail_information: Option<Vec<GuardrailEntry>>,
    #[serde(rename = "standard_logging_guardrail_information")]
    pub standard_logging: Option<Vec<GuardrailEntry>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MetricsKey {
    pub guardrail_id: String,
    pub date: String,
}

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UsageUnitKey {
    pub guardrail_id: String,
    pub date: String,
    pub team_id: String,
    pub api_key: String,
    pub usage_unit: String,
}

// the key holds the caller's api key, so a diagnostic shows the other fields and leaves that one out
impl fmt::Debug for UsageUnitKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsageUnitKey")
            .field("guardrail_id", &self.guardrail_id)
            .field("date", &self.date)
            .field("team_id", &self.team_id)
            .field("usage_unit", &self.usage_unit)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GuardrailCounts {
    pub requests_evaluated: i64,
    pub passed_count: i64,
    pub blocked_count: i64,
    pub flagged_count: i64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UsageUnitIncrement {
    pub units: i64,
    pub cost: Decimal,
    pub untracked_units: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct GuardrailIndexRow {
    pub request_id: String,
    pub guardrail_id: String,
    pub policy_id: Option<String>,
    pub start_time: String,
    #[serde(skip)]
    pub instant: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RollupRow {
    Metrics {
        key: MetricsKey,
        counts: GuardrailCounts,
    },
    Units {
        key: UsageUnitKey,
        increment: UsageUnitIncrement,
    },
}

impl RollupRow {
    #[must_use]
    pub(crate) fn rows(
        metrics: IndexMap<MetricsKey, GuardrailCounts>,
        units: IndexMap<UsageUnitKey, UsageUnitIncrement>,
    ) -> Vec<Self> {
        let mut rows: Vec<Self> = metrics
            .into_iter()
            .map(|(key, counts)| Self::Metrics { key, counts })
            .collect();
        rows.extend(
            units
                .into_iter()
                .map(|(key, increment)| Self::Units { key, increment }),
        );
        rows
    }

    pub(crate) async fn write<Sink>(&self, sink: &Sink) -> Result<(), Sink::Error>
    where
        Sink: GuardrailRollupSink,
    {
        match self {
            Self::Metrics { key, counts } => sink.upsert_metrics(key, *counts).await,
            Self::Units { key, increment } => sink.upsert_units(key, increment).await,
        }
    }

    #[must_use]
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            Self::Metrics { .. } => METRICS_LABEL,
            Self::Units { .. } => UNITS_LABEL,
        }
    }
}

pub trait GuardrailRollupFailure: StdError + Send + Sync + 'static {
    fn is_retry_safe(&self) -> bool;
}

pub type RollupFuture<'sink, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'sink>>;

pub trait GuardrailRollupSink: Send + Sync {
    type Error: GuardrailRollupFailure;

    fn create_index_rows<'rows>(
        &'rows self,
        rows: &'rows [GuardrailIndexRow],
    ) -> RollupFuture<'rows, Self::Error>;

    fn upsert_metrics<'row>(
        &'row self,
        key: &'row MetricsKey,
        counts: GuardrailCounts,
    ) -> RollupFuture<'row, Self::Error>;

    fn upsert_units<'row>(
        &'row self,
        key: &'row UsageUnitKey,
        increment: &'row UsageUnitIncrement,
    ) -> RollupFuture<'row, Self::Error>;
}

pub type SharedPendingRollups = Arc<Mutex<PendingRollups>>;

type FreshRollups = (
    IndexMap<MetricsKey, GuardrailCounts>,
    IndexMap<UsageUnitKey, UsageUnitIncrement>,
);

// LiteLLM's process_spend_logs_guardrail_usage never raises: every failed row is logged or queued for the next flush
pub async fn process_guardrail_usage<Sink>(
    payloads: &[GuardrailSpendLog],
    pending: &SharedPendingRollups,
    sink: &Sink,
    limits: StatementLimits,
    pause: &dyn RetryPause,
) where
    Sink: GuardrailRollupSink,
{
    if payloads.is_empty() {
        return;
    }
    let fresh = (
        rollups::fresh_metrics(payloads),
        rollups::fresh_units(payloads),
    );
    let index = index_rows::build(payloads);
    let (metrics, units) = merged_with_pending(pending, fresh);
    if metrics.is_empty() && units.is_empty() && index.is_empty() {
        return;
    }
    flush::index_batches(sink, &index, limits).await;
    let failed = flush::rows(sink, RollupRow::rows(metrics, units), pause).await;
    if !failed.is_empty() {
        requeue_failed(pending, failed);
    }
}

// Takes the queued rows before the write, then merges the fresh rows into insertion order
fn merged_with_pending(pending: &SharedPendingRollups, fresh: FreshRollups) -> FreshRollups {
    let mut state = pending.lock().unwrap_or_else(PoisonError::into_inner);
    let queued_metrics = mem::take(&mut state.metrics);
    let queued_units = mem::take(&mut state.units);
    (
        rollups::merged_rows(queued_metrics, fresh.0, rollups::summed_counts),
        rollups::merged_rows(queued_units, fresh.1, rollups::summed_units),
    )
}

// Keeps only the rows whose retries ran out, merged with whatever a concurrent flush queued meanwhile
fn requeue_failed(pending: &SharedPendingRollups, failed: Vec<RollupRow>) {
    let (failed_metrics, failed_units) = rollups::split(failed);
    let mut state = pending.lock().unwrap_or_else(PoisonError::into_inner);
    let queued_metrics = mem::take(&mut state.metrics);
    let queued_units = mem::take(&mut state.units);
    state.metrics = rollups::capped(
        rollups::merged_rows(queued_metrics, failed_metrics, rollups::summed_counts),
        METRICS_LABEL,
    );
    state.units = rollups::capped(
        rollups::merged_rows(queued_units, failed_units, rollups::summed_units),
        UNITS_LABEL,
    );
}
