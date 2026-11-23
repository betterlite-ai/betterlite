mod impls;

use std::sync::{Mutex, PoisonError};

use betterlite_server::shadow_eval_lifecycle::store::{
    LegRow, TargetKeyRow, TargetRows, TargetTeamRow, TargetUserRow,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

use super::forms::{Input, leg_state};
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
    pub(super) fn build(input: &Input) -> Result<Self, OperationError> {
        let targets = TargetRows {
            keys: input
                .rows
                .keys
                .iter()
                .map(|seed| TargetKeyRow {
                    token: seed.token.clone(),
                    key_alias: seed.key_alias.clone(),
                    key_name: seed.key_name.clone(),
                    team_id: seed.team_id.clone(),
                })
                .collect(),
            teams: input
                .rows
                .teams
                .iter()
                .map(|seed| TargetTeamRow {
                    team_id: seed.team_id.clone(),
                    team_alias: seed.team_alias.clone(),
                })
                .collect(),
            users: input
                .rows
                .users
                .iter()
                .map(|seed| TargetUserRow {
                    user_id: seed.user_id.clone(),
                    user_email: seed.user_email.clone(),
                })
                .collect(),
        };
        let mut legs = Vec::with_capacity(input.jobs.len());
        for seed in &input.jobs {
            legs.push(LegRow {
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
                stopped_at: match seed.stopped_at_millis {
                    Some(millis) => Some(timestamp(millis)?),
                    None => None,
                },
                stopped_by: seed.stopped_by.clone(),
            });
        }
        let mut attempts = Vec::with_capacity(input.attempts.len());
        for seed in &input.attempts {
            attempts.push(ProbeAttempt {
                job_id: seed.job_id.clone(),
                created_at: timestamp(seed.created_at_millis)?,
                spend: seed.spend.parse::<Decimal>().map_err(invalid)?,
            });
        }
        Ok(Self {
            targets,
            now: timestamp(input.now_millis)?,
            legs: Mutex::new(legs),
            attempts,
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
        match leg.max_budget {
            Some(budget) => self.attempt_spend(leg) < budget,
            None => true,
        }
    }
}

fn within(attempt: &ProbeAttempt, leg: &LegRow) -> bool {
    match leg.stopped_at {
        Some(stamp) => attempt.created_at <= stamp,
        None => true,
    }
}

fn parse_budget(seed: Option<&str>) -> Result<Option<Decimal>, OperationError> {
    match seed {
        Some(text) => Ok(Some(text.parse::<Decimal>().map_err(invalid)?)),
        None => Ok(None),
    }
}
