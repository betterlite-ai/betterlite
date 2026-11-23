mod impls;

use std::sync::{Mutex, PoisonError};

use betterlite_server::shadow_eval_lifecycle::store::{
    TargetKeyRow, TargetRows, TargetTeamRow, TargetUserRow,
};
use betterlite_server::shadow_result_aggregation::legs::LegRow;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

use super::forms::{JobSeed, Payload};
use super::output::leg_state;
use super::{invalid, timestamp};
use crate::router_probes::error::OperationError;

pub(super) struct ProbeAttempt {
    job_id: String,
    created_at: DateTime<Utc>,
    spend: Decimal,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct ProbeStoreError {
    pub(super) message: String,
}

// an in-memory LiteLLM_ShadowEvalJob table running the same sweep, claim, insert and stop statements
pub(super) struct ProbeStore {
    targets: TargetRows,
    now: DateTime<Utc>,
    legs: Mutex<Vec<LegRow>>,
    attempts: Vec<ProbeAttempt>,
    seeds_fail: bool,
    create_conflict: bool,
}

impl ProbeStore {
    pub(super) fn build(input: &Payload) -> Result<Self, OperationError> {
        Ok(Self {
            targets: target_rows(input),
            now: timestamp(input.now_millis)?,
            legs: Mutex::new(leg_rows(input)?),
            attempts: attempt_rows(input)?,
            seeds_fail: input.seeds_fail,
            create_conflict: input.create_conflict,
        })
    }

    pub(super) fn states(&self) -> Vec<String> {
        let mut states: Vec<String> = {
            let legs = self.legs.lock().unwrap_or_else(PoisonError::into_inner);
            legs.iter().map(leg_state).collect()
        };
        states.sort();
        states
    }

    fn legs_of(&self, group_id: &str) -> Vec<LegRow> {
        let legs = self.legs.lock().unwrap_or_else(PoisonError::into_inner);
        let matching = legs.iter().filter(|leg| leg.group_id == group_id);
        matching.cloned().collect()
    }

    fn attempt_count(&self, leg: &LegRow) -> i64 {
        let live = self
            .attempts
            .iter()
            .filter(|attempt| attempt.job_id == leg.id && within(attempt, leg));
        i64::try_from(live.count()).unwrap_or(i64::MAX)
    }

    fn attempt_spend(&self, leg: &LegRow) -> Decimal {
        self.attempts
            .iter()
            .filter(|attempt| attempt.job_id == leg.id && within(attempt, leg))
            .map(|attempt| attempt.spend)
            .sum()
    }

    fn budget_open(&self, leg: &LegRow) -> bool {
        leg.max_budget
            .is_none_or(|budget| self.attempt_spend(leg) < budget)
    }
}

fn target_rows(input: &Payload) -> TargetRows {
    TargetRows {
        keys: key_rows(input),
        teams: team_rows(input),
        users: user_rows(input),
    }
}

fn key_rows(input: &Payload) -> Vec<TargetKeyRow> {
    input
        .rows
        .keys
        .iter()
        .map(|seed| TargetKeyRow {
            token: seed.token.clone(),
            key_alias: seed.key_alias.clone(),
            key_name: seed.key_name.clone(),
            team_id: seed.team_id.clone(),
        })
        .collect()
}

fn team_rows(input: &Payload) -> Vec<TargetTeamRow> {
    input
        .rows
        .teams
        .iter()
        .map(|seed| TargetTeamRow {
            team_id: seed.team_id.clone(),
            team_alias: seed.team_alias.clone(),
        })
        .collect()
}

fn user_rows(input: &Payload) -> Vec<TargetUserRow> {
    input
        .rows
        .users
        .iter()
        .map(|seed| TargetUserRow {
            user_id: seed.user_id.clone(),
            user_email: seed.user_email.clone(),
        })
        .collect()
}

fn leg_rows(input: &Payload) -> Result<Vec<LegRow>, OperationError> {
    let mut rows = Vec::with_capacity(input.jobs.len());
    for seed in &input.jobs {
        rows.push(leg_row(seed)?);
    }
    Ok(rows)
}

fn leg_row(seed: &JobSeed) -> Result<LegRow, OperationError> {
    Ok(LegRow {
        id: seed.id.clone(),
        group_id: seed.group_id.clone(),
        target_type: seed.target_type,
        target_id: seed.target_id.clone(),
        router_name: seed.router_name.clone(),
        router_names: seed.router_names.clone(),
        models: seed.models.clone(),
        direction: seed.direction,
        baseline_model: seed.baseline_model.clone(),
        judge_model: seed.judge_model.clone(),
        shadow_percentage: seed.shadow_percentage.parse::<f64>().map_err(invalid)?,
        max_turns: seed.max_turns,
        max_budget: parse_budget(seed.max_budget.as_deref())?,
        created_at: timestamp(seed.created_at_millis)?,
        ends_at: timestamp(seed.ends_at_millis)?,
        stopped_at: seed.stopped_at_millis.map(timestamp).transpose()?,
        stopped_by: seed.stopped_by.clone(),
    })
}

fn attempt_rows(input: &Payload) -> Result<Vec<ProbeAttempt>, OperationError> {
    let mut rows = Vec::with_capacity(input.attempts.len());
    for seed in &input.attempts {
        rows.push(ProbeAttempt {
            job_id: seed.job_id.clone(),
            created_at: timestamp(seed.created_at_millis)?,
            spend: seed.spend.parse::<Decimal>().map_err(invalid)?,
        });
    }
    Ok(rows)
}

fn within(attempt: &ProbeAttempt, leg: &LegRow) -> bool {
    leg.stopped_at
        .is_none_or(|stamp| attempt.created_at <= stamp)
}

fn parse_budget(seed: Option<&str>) -> Result<Option<Decimal>, OperationError> {
    seed.map(|text| text.parse::<Decimal>().map_err(invalid))
        .transpose()
}
