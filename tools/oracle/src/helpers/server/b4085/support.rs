use std::collections::VecDeque;
use std::future::ready;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::current_spend::SpendQuery;
use betterlite_core::shadow_evidence::records::{CallFailure, JudgeVerdict, ShadowResponse};
use betterlite_db::shadow_tables::rows::{
    FunnelCounters, FunnelStage, JobAttemptStats, ShadowAttemptRow, ShadowFuture, ShadowJobRow,
    ShadowTables,
};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::shadow_calls::unmask::unmask_preference;
use betterlite_server::shadow_calls::{JudgeRequest, ShadowRequest};
use betterlite_server::shadow_eval_job_loading::attempt::{SpendCounter, SpendFuture};
use betterlite_server::shadow_evaluation::advisory_budget::{CachedTeams, TeamFuture};
use betterlite_server::shadow_evaluation::arm::{ArmCalls, ArmFailure, ArmStage};
use betterlite_server::shadow_evaluation::pipeline::{
    FunnelRecorder, JobSpendFuture, JobSpendReader,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::inputs::PipelineCall;

// a poisoned lock is a failed test, not a corrupt answer, so the guard comes back rather than unwrapping again
pub(crate) fn locked<Items>(log: &Mutex<Items>) -> MutexGuard<'_, Items> {
    match log.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("scripted database failure")]
pub(crate) struct Scripted;

#[derive(Clone, Copy)]
pub(crate) enum SpendScript {
    Value(Decimal),
    Unreadable,
}

pub(crate) struct SpendFake {
    pub(crate) key: SpendScript,
    pub(crate) team: SpendScript,
    pub(crate) reads: Mutex<Vec<Value>>,
}

#[async_trait]
impl SpendReader for SpendFake {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        let maximum = query.max_budget.map(|value| value.normalize().to_string());
        locked(&self.reads).push(json!([
            query.counter_key,
            query.fallback_spend.normalize().to_string(),
            maximum
        ]));
        let script = if query.counter_key.starts_with("spend:key:") {
            self.key
        } else {
            self.team
        };
        match script {
            SpendScript::Value(spend) => Ok(spend),
            SpendScript::Unreadable => Err(SpendReadError::Unverifiable {
                counter_key: query.counter_key.to_owned(),
            }),
        }
    }
}

pub(crate) struct TeamsFake {
    pub(crate) team: Option<LiteLLMTeamTableCachedObj>,
}

impl CachedTeams for TeamsFake {
    fn cached_team<'lookup>(&'lookup self, _team_id: &'lookup str) -> TeamFuture<'lookup> {
        Box::pin(ready(match &self.team {
            Some(team) => Ok(team.clone()),
            None => Err(TeamLookupError::NoDatabase),
        }))
    }
}

// the database, the spend counter and the funnel are one recorder here: every stage's write lands in the same answer
#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) rows: Mutex<Vec<ShadowAttemptRow>>,
    pub(crate) writes: Mutex<Vec<String>>,
    pub(crate) events: Mutex<Vec<(String, FunnelStage)>>,
}

impl FunnelRecorder for Recorder {
    fn record(&self, job_id: &str, stage: FunnelStage) {
        locked(&self.events).push((job_id.to_owned(), stage));
    }
}

impl ShadowTables for Recorder {
    type Error = Scripted;

    fn find_active_jobs(
        &self,
        _now: DateTime<Utc>,
    ) -> ShadowFuture<'_, Vec<ShadowJobRow>, Scripted> {
        Box::pin(ready(Err(Scripted)))
    }

    fn attempt_stats<'database>(
        &'database self,
        _job_ids: &'database [String],
    ) -> ShadowFuture<'database, Vec<JobAttemptStats>, Scripted> {
        Box::pin(ready(Err(Scripted)))
    }

    fn insert_attempt<'database>(
        &'database self,
        attempt: &'database ShadowAttemptRow,
    ) -> ShadowFuture<'database, (), Scripted> {
        let row = attempt.clone();
        Box::pin(async move {
            locked(&self.rows).push(row);
            Ok(())
        })
    }

    fn upsert_funnel<'database>(
        &'database self,
        _job_id: &'database str,
        _counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), Scripted> {
        Box::pin(ready(Ok(())))
    }
}

impl SpendCounter for Recorder {
    type Error = Scripted;

    fn add_spend<'counter>(
        &'counter self,
        counter_key: &'counter str,
        cost: Decimal,
    ) -> SpendFuture<'counter, (), Scripted> {
        let entry = format!("write:{counter_key}:{}", cost.normalize());
        Box::pin(async move {
            locked(&self.writes).push(entry);
            Ok(())
        })
    }
}

pub(crate) struct JobSpendFake {
    pub(crate) spend: Decimal,
    pub(crate) reads: Mutex<Vec<Value>>,
}

impl JobSpendReader for JobSpendFake {
    fn read_spend<'reader>(
        &'reader self,
        counter_key: &'reader str,
        fallback_spend: Decimal,
        max_budget: Decimal,
    ) -> JobSpendFuture<'reader> {
        locked(&self.reads).push(json!([
            counter_key,
            fallback_spend.normalize().to_string(),
            max_budget.normalize().to_string()
        ]));
        Box::pin(ready(Ok(self.spend)))
    }
}

enum ShadowAnswer {
    Reply,
    Empty,
    Raised,
}

enum JudgeAnswer {
    Preference(String),
    Invalid,
}

pub(crate) struct CallsFake {
    shadow_answers: Mutex<VecDeque<ShadowAnswer>>,
    judge_answer: JudgeAnswer,
}

impl CallsFake {
    pub(crate) fn new(call: &PipelineCall) -> Self {
        let mut shadow_answers = VecDeque::new();
        for (index, arm) in call.arms.iter().enumerate() {
            if index == 0 && call.pipeline_error {
                shadow_answers.push_back(ShadowAnswer::Raised);
            } else if arm.shadow == "reply" {
                shadow_answers.push_back(ShadowAnswer::Reply);
            } else {
                shadow_answers.push_back(ShadowAnswer::Empty);
            }
        }
        let judge_answer = match call.judge_reply.as_str() {
            "a" => JudgeAnswer::Preference("A".to_owned()),
            "b" => JudgeAnswer::Preference("B".to_owned()),
            "tie" => JudgeAnswer::Preference("tie".to_owned()),
            _ => JudgeAnswer::Invalid,
        };
        Self {
            shadow_answers: Mutex::new(shadow_answers),
            judge_answer,
        }
    }
}

fn call_failure(message: &str) -> CallFailure {
    CallFailure {
        error: message.to_owned(),
        cost: Decimal::new(75, 2),
        classifier_cost: Decimal::ZERO,
    }
}

#[async_trait]
impl ArmCalls for CallsFake {
    async fn shadow(&self, _request: &ShadowRequest<'_>) -> ArmStage<ShadowResponse> {
        let mut answers = locked(&self.shadow_answers);
        match answers.pop_front() {
            Some(ShadowAnswer::Reply) => Ok(ShadowResponse {
                text: "shadow answer".to_owned(),
                model: "gpt-4o-mini".to_owned(),
                tier: None,
                cost: Decimal::new(75, 2),
                classifier_cost: Decimal::ZERO,
            }),
            Some(ShadowAnswer::Empty) => Err(ArmFailure::Failed(call_failure(
                "the shadow reply was empty",
            ))),
            Some(ShadowAnswer::Raised) => Err(ArmFailure::Raised("provider exploded".to_owned())),
            None => Err(ArmFailure::Raised(
                "no scripted shadow answer left".to_owned(),
            )),
        }
    }

    async fn judge(&self, _request: &JudgeRequest<'_>) -> ArmStage<JudgeVerdict> {
        // the oracle cases seed LiteLLM's A/B coin so a raw A unmasked to the shadow preference
        match &self.judge_answer {
            JudgeAnswer::Preference(raw) => Ok(JudgeVerdict {
                preference: unmask_preference(raw, false),
                confidence: Decimal::new(9, 1),
                cost: Decimal::new(75, 2),
            }),
            JudgeAnswer::Invalid => Err(ArmFailure::Failed(call_failure(
                "the judge verdict was not valid JSON",
            ))),
        }
    }
}

pub(crate) fn failure(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

pub(crate) fn build_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| failure("RuntimeError", &error.to_string()))
}
