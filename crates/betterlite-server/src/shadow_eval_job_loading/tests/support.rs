#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::shadow_tables::rows::{
    FunnelCounters, JobAttemptStats, ShadowAttemptRow, ShadowFuture, ShadowJobRow, ShadowTables,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::super::attempt::{SpendCounter, SpendFuture};
use super::super::cache::Clock;
use super::super::jobs::{ActiveShadowEvalJob, ShadowEvalDirection};

#[derive(Debug, thiserror::Error)]
#[error("scripted database failure")]
pub(crate) struct Scripted;

pub(crate) type EventLog = Arc<Mutex<Vec<String>>>;

pub(crate) fn event_log() -> EventLog {
    Arc::new(Mutex::new(Vec::new()))
}

pub(crate) fn frozen_clock(seconds: Arc<Mutex<f64>>) -> Clock {
    Arc::new(move || *seconds.lock().unwrap_or_else(PoisonError::into_inner))
}

#[derive(Default)]
pub(crate) struct TablesState {
    pub(crate) rows: Vec<ShadowJobRow>,
    pub(crate) stats: Vec<JobAttemptStats>,
    pub(crate) find_calls: usize,
    pub(crate) stats_requests: Vec<Vec<String>>,
    pub(crate) inserts: Vec<ShadowAttemptRow>,
    pub(crate) fail_find: bool,
    pub(crate) fail_stats: bool,
    pub(crate) fail_insert: bool,
}

#[derive(Default)]
pub(crate) struct FakeTables {
    pub(crate) state: Mutex<TablesState>,
    pub(crate) events: EventLog,
}

#[derive(Default)]
pub(crate) struct CounterState {
    pub(crate) fail_writes: bool,
    pub(crate) spend: Decimal,
}

#[derive(Default)]
pub(crate) struct FakeCounter {
    pub(crate) state: Mutex<CounterState>,
    pub(crate) events: EventLog,
}

// one log across both fakes is what proves the counter write lands before the row insert
pub(crate) fn tables_and_counter() -> (FakeTables, FakeCounter) {
    let log = event_log();
    let tables = FakeTables {
        state: Mutex::new(TablesState::default()),
        events: Arc::clone(&log),
    };
    let counter = FakeCounter {
        state: Mutex::new(CounterState::default()),
        events: log,
    };
    (tables, counter)
}

pub(crate) fn fake_tables(rows: Vec<ShadowJobRow>) -> FakeTables {
    let mut state = TablesState::default();
    state.rows = rows;
    FakeTables {
        state: Mutex::new(state),
        events: event_log(),
    }
}

pub(crate) fn log_entries(log: &EventLog) -> Vec<String> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub(crate) fn job_row(direction: &str, baseline_model: Option<&str>) -> ShadowJobRow {
    ShadowJobRow {
        id: "job-1".to_owned(),
        target_type: "key".to_owned(),
        target_id: "key-1".to_owned(),
        router_name: "router-a".to_owned(),
        router_names: Vec::new(),
        models: Vec::new(),
        direction: direction.to_owned(),
        baseline_model: baseline_model.map(str::to_owned),
        shadow_percentage: 100.0,
        judge_model: "judge-model".to_owned(),
        max_turns: 5,
        max_budget: None,
        ends_at: Utc::now(),
    }
}

pub(crate) fn active_job(
    id: &str,
    direction: ShadowEvalDirection,
    shadow_percentage: f64,
) -> ActiveShadowEvalJob {
    ActiveShadowEvalJob {
        id: id.to_owned(),
        router_name: "router-a".to_owned(),
        arm_router_names: vec!["router-a".to_owned()],
        models: BTreeSet::new(),
        direction,
        baseline_model: None,
        shadow_percentage,
        judge_model: "judge-model".to_owned(),
        max_turns: 5,
        max_budget: None,
        ends_at: Utc::now(),
        attempts: 0,
        spend: Decimal::ZERO,
    }
}

impl FakeTables {
    fn record_attempt(&self, attempt: &ShadowAttemptRow) -> Result<(), Scripted> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("insert:{}", attempt.request_id));
        let failed = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.inserts.push(attempt.clone());
            state.fail_insert
        };
        if failed {
            return Err(Scripted);
        }
        Ok(())
    }
}

impl ShadowTables for FakeTables {
    type Error = Scripted;

    fn find_active_jobs(
        &self,
        _now: DateTime<Utc>,
    ) -> ShadowFuture<'_, Vec<ShadowJobRow>, Self::Error> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.find_calls = state.find_calls.saturating_add(1);
            if state.fail_find {
                return Err(Scripted);
            }
            Ok(state.rows.clone())
        })
    }

    fn attempt_stats<'database>(
        &'database self,
        job_ids: &'database [String],
    ) -> ShadowFuture<'database, Vec<JobAttemptStats>, Self::Error> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.stats_requests.push(job_ids.to_vec());
            if state.fail_stats {
                return Err(Scripted);
            }
            Ok(state.stats.clone())
        })
    }

    fn insert_attempt<'database>(
        &'database self,
        attempt: &'database ShadowAttemptRow,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async move { self.record_attempt(attempt) })
    }

    fn upsert_funnel<'database>(
        &'database self,
        _job_id: &'database str,
        _counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async { Ok::<(), Scripted>(()) })
    }
}

impl SpendCounter for FakeCounter {
    type Error = Scripted;

    fn add_spend<'counter>(
        &'counter self,
        counter_key: &'counter str,
        cost: Decimal,
    ) -> SpendFuture<'counter, (), Self::Error> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("write:{counter_key}:{cost}"));
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.fail_writes {
                return Err(Scripted);
            }
            state.spend = state.spend.saturating_add(cost);
            drop(state);
            Ok(())
        })
    }
}
