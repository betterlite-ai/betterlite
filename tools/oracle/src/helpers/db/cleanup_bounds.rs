use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::spend_storage::retention::cleanup_defaults;
use betterlite_core::duration::error::DurationError;
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_db::spend_retention::batches::cutoff::Cutoff;
use betterlite_db::spend_retention::batches::deadline::{group_deadline, statement_timeout_ms};
use betterlite_db::spend_retention::batches::settings::{
    CleanupGeneralSettings, CleanupSettings, DurationParser, Retention,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::{DateTime, NaiveDate, Utc};
use jiff::tz;
use tokio::time::Instant;

const RETENTIONS: [Retention; 4] = [
    Retention::SpendLogs,
    Retention::AutorouterSessions,
    Retention::HealthChecks,
    Retention::DailyTagSpend,
];

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    general_settings: CleanupGeneralSettings,
    retention: Vec<String>,
    now_ms: u64,
    deadlines_ms: Vec<u64>,
    groups: Vec<GroupInput>,
    timestamps: Vec<DateTime<Utc>>,
    days: Vec<NaiveDate>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupInput {
    overall_ms: u64,
    groups: u32,
}

// month lengths depend on the clock, so the cases stay with fixed units and any instant serves
struct CoreDurations;

impl DurationParser for CoreDurations {
    type Failure = DurationError;

    fn duration_seconds(&self, spec: &str) -> Result<i64, DurationError> {
        duration_in_seconds(spec, DateTime::<Utc>::UNIX_EPOCH, None, tz::db())
            .map(|parsed| parsed.seconds)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = CleanupSettings {
            general: input.general_settings.clone(),
            defaults: cleanup_defaults(&BTreeMap::<String, String>::new()).or_config_error()?,
        };
        let bounds = settings.bounds(&CoreDurations);
        let mut retention = Vec::new();
        for key in &input.retention {
            let group = RETENTIONS
                .into_iter()
                .find(|group| group.key() == key.as_str())
                .ok_or("unknown retention setting")
                .or_value_error()?;
            retention.push(settings.retention_seconds(group, &CoreDurations));
        }
        let (timeouts, group_timeouts) = deadline_timeouts(input, bounds.batch_timeout)?;
        let cutoffs: Vec<[String; 2]> = input
            .timestamps
            .iter()
            .map(|moment| Cutoff::Timestamp(*moment))
            .chain(input.days.iter().map(|day| Cutoff::Day(*day)))
            .map(|cutoff| [cutoff.cast().to_owned(), cutoff.text()])
            .collect();
        Ok(serde_json::json!({
            "bounds": [
                bounds.batch_size,
                bounds.max_batches,
                u64::try_from(bounds.run_budget.as_millis()).or_value_error()?,
                u64::try_from(bounds.batch_timeout.as_millis()).or_value_error()?,
            ],
            "retention": retention,
            "timeouts_ms": timeouts,
            "group_timeouts_ms": group_timeouts,
            "cutoffs": cutoffs,
        }))
    }
}

// _timeout_ms of each deadline, then of each _group_deadline share, all read at the case's patched now
fn deadline_timeouts(
    input: &Input,
    batch_timeout: Duration,
) -> Result<(Vec<u64>, Vec<u64>), Box<HelperError>> {
    let base = Instant::now();
    let now = after(base, input.now_ms)?;
    let mut direct = Vec::new();
    for deadline_ms in &input.deadlines_ms {
        let deadline = after(base, *deadline_ms)?;
        direct.push(statement_timeout_ms(now, deadline, batch_timeout));
    }
    let mut shared = Vec::new();
    for group in &input.groups {
        let deadline = group_deadline(after(base, group.overall_ms)?, now, group.groups);
        shared.push(statement_timeout_ms(now, deadline, batch_timeout));
    }
    Ok((direct, shared))
}

// the oracle's monotonic readings are offsets from one base instant, as the Python side patches time.monotonic
fn after(base: Instant, millis: u64) -> Result<Instant, Box<HelperError>> {
    base.checked_add(Duration::from_millis(millis))
        .ok_or("the offset overflows the clock")
        .or_value_error()
}
