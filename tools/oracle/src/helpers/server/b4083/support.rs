use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::shadow_tables::rows::{
    AttemptOutcome, FunnelCounters, JobAttemptStats, ShadowAttemptRow, ShadowFuture, ShadowJobRow,
    ShadowTables,
};
use betterlite_server::shadow_eval_job_loading::attempt::{SpendCounter, SpendFuture};
use betterlite_server::shadow_eval_job_loading::jobs::ActiveShadowEvalJob;
use betterlite_server::shadow_eval_job_loading::loader::LoadedJobs;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use crate::helpers::HelperError;

use super::inputs::{JobInput, StatInput};

#[derive(Debug, thiserror::Error)]
#[error("scripted database failure")]
pub(crate) struct Scripted;

pub(crate) struct StoredJob {
    stopped_at: Option<DateTime<Utc>>,
    row: ShadowJobRow,
}

pub(crate) struct LoadTables {
    rows: Vec<StoredJob>,
    stats: Vec<JobAttemptStats>,
    pub(crate) find_calls: AtomicUsize,
    pub(crate) stats_requests: Mutex<Vec<Vec<String>>>,
}

impl LoadTables {
    pub(crate) fn new(rows: &[JobInput], stats: &[StatInput]) -> Self {
        let rows = rows
            .iter()
            .map(|job| StoredJob {
                stopped_at: job.stopped_at,
                row: job.to_row(),
            })
            .collect();
        let stats = stats
            .iter()
            .map(|stat| JobAttemptStats {
                job_id: stat.job_id.clone(),
                attempts: stat.attempts,
                spend: stat.spend,
            })
            .collect();
        Self {
            rows,
            stats,
            find_calls: AtomicUsize::new(0),
            stats_requests: Mutex::new(Vec::new()),
        }
    }
}

impl ShadowTables for LoadTables {
    type Error = Scripted;

    fn find_active_jobs(
        &self,
        now: DateTime<Utc>,
    ) -> ShadowFuture<'_, Vec<ShadowJobRow>, Self::Error> {
        Box::pin(async move {
            self.find_calls.fetch_add(1, Ordering::Relaxed);
            Ok(self
                .rows
                .iter()
                .filter(|stored| stored.stopped_at.is_none() && stored.row.ends_at > now)
                .map(|stored| stored.row.clone())
                .collect())
        })
    }

    fn attempt_stats<'database>(
        &'database self,
        job_ids: &'database [String],
    ) -> ShadowFuture<'database, Vec<JobAttemptStats>, Self::Error> {
        Box::pin(async move {
            self.stats_requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(job_ids.to_vec());
            Ok(self
                .stats
                .iter()
                .filter(|stat| job_ids.contains(&stat.job_id))
                .cloned()
                .collect())
        })
    }

    fn insert_attempt<'database>(
        &'database self,
        _attempt: &'database ShadowAttemptRow,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async { Err(Scripted) })
    }

    fn upsert_funnel<'database>(
        &'database self,
        _job_id: &'database str,
        _counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async { Err(Scripted) })
    }
}

pub(crate) struct AttemptTables {
    pub(crate) events: Arc<Mutex<Vec<String>>>,
    pub(crate) inserts: Mutex<Vec<ShadowAttemptRow>>,
}

impl AttemptTables {
    fn record(&self, attempt: &ShadowAttemptRow) -> Result<(), Scripted> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("insert:{}", attempt.request_id));
        self.inserts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ShadowAttemptRow::clone(attempt));
        Ok(())
    }
}

impl ShadowTables for AttemptTables {
    type Error = Scripted;

    fn find_active_jobs(
        &self,
        _now: DateTime<Utc>,
    ) -> ShadowFuture<'_, Vec<ShadowJobRow>, Self::Error> {
        Box::pin(async { Err(Scripted) })
    }

    fn attempt_stats<'database>(
        &'database self,
        _job_ids: &'database [String],
    ) -> ShadowFuture<'database, Vec<JobAttemptStats>, Self::Error> {
        Box::pin(async { Err(Scripted) })
    }

    fn insert_attempt<'database>(
        &'database self,
        attempt: &'database ShadowAttemptRow,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async move { self.record(attempt) })
    }

    fn upsert_funnel<'database>(
        &'database self,
        _job_id: &'database str,
        _counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async { Ok(()) })
    }
}

pub(crate) struct FakeCounter {
    pub(crate) events: Arc<Mutex<Vec<String>>>,
}

impl SpendCounter for FakeCounter {
    type Error = Scripted;

    fn add_spend<'counter>(
        &'counter self,
        counter_key: &'counter str,
        cost: Decimal,
    ) -> SpendFuture<'counter, (), Self::Error> {
        Box::pin(async move {
            let entry = format!("write:{counter_key}:{}", cost.normalize());
            self.events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(entry);
            Ok(())
        })
    }
}

pub(crate) fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

pub(crate) fn build_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| failure("RuntimeError", error))
}

fn targets(metadata: &BTreeMap<String, Value>) -> Vec<(String, String)> {
    let mut resolved = Vec::new();
    for (target_type, key) in [
        ("key", "user_api_key_hash"),
        ("team", "user_api_key_team_id"),
        ("user", "user_api_key_user_id"),
    ] {
        if let Some(value) = metadata.get(key).and_then(Value::as_str) {
            if !value.is_empty() {
                resolved.push((target_type.to_owned(), value.to_owned()));
            }
        }
    }
    resolved
}

pub(crate) fn candidates<'jobs>(
    loaded: &'jobs LoadedJobs,
    metadata: &BTreeMap<String, Value>,
) -> Vec<&'jobs ActiveShadowEvalJob> {
    let mut candidates = Vec::new();
    for target in targets(metadata) {
        if let Some(jobs) = loaded.jobs.get(&target) {
            candidates.extend(jobs.iter());
        }
    }
    candidates
}

pub(crate) fn outcome(raw: &str) -> Result<AttemptOutcome, Box<HelperError>> {
    match raw {
        "real" => Ok(AttemptOutcome::Real),
        "shadow" => Ok(AttemptOutcome::Shadow),
        "tie" => Ok(AttemptOutcome::Tie),
        "error" => Ok(AttemptOutcome::Error),
        other => Err(failure("InvalidInput", format!("unknown outcome {other}"))),
    }
}
