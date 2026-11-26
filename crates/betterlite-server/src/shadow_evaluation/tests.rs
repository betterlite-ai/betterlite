#![cfg(test)]

// fakes shared by the advisory, arm and pipeline tests; there is no #[test] here, the check filters select the three submodules
use std::collections::{BTreeSet, VecDeque};
use std::future::ready;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::auth_key_budgets::alerts::{BudgetAlert, BudgetAlertSink};
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::current_spend::SpendQuery;
use betterlite_auth::team_budget_admission::alerts::{TeamAlert, TeamAlertSink};
use betterlite_core::shadow_evidence::records::{CallFailure, JudgeVerdict, ShadowResponse};
use betterlite_db::shadow_tables::rows::{
    FunnelCounters, FunnelStage, JobAttemptStats, ShadowAttemptRow, ShadowFuture, ShadowJobRow,
    ShadowTables,
};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use chrono::{DateTime, TimeZone as _, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::shadow_calls::{JudgeRequest, ShadowRequest};
use crate::shadow_eval_job_loading::attempt::{SpendCounter, SpendFuture};
use crate::shadow_eval_job_loading::jobs::{ActiveShadowEvalJob, ShadowEvalDirection};
use crate::shadow_evaluation::ShadowSample;
use crate::shadow_evaluation::advisory_budget::{CachedTeams, TeamFuture};
use crate::shadow_evaluation::arm::{ArmCalls, ArmFailure, ArmStage};
use crate::shadow_evaluation::pipeline::{FunnelRecorder, JobSpendFuture, JobSpendReader};

pub(crate) const TOKEN: &str = "token-1";

#[derive(Debug, thiserror::Error)]
#[error("the fake collaborator failed")]
pub(crate) struct TestError;

pub(crate) fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap()
}

// a key's user_api_key_auth record as the success hook carries it; None omits the field entirely
pub(crate) fn key_auth(max_budget: Option<&str>, spend: &str, team_id: Option<&str>) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("token".to_owned(), Json::Text(TOKEN.to_owned()));
    fields.insert("spend".to_owned(), json(spend));
    if let Some(maximum) = max_budget {
        fields.insert("max_budget".to_owned(), json(maximum));
    }
    if let Some(team) = team_id {
        fields.insert("team_id".to_owned(), Json::Text(team.to_owned()));
    }
    Json::Object(fields)
}

pub(crate) fn add_field(object: &mut Json, name: &str, value: Json) {
    let Json::Object(fields) = object else {
        panic!("the fixture is an object");
    };
    fields.insert(name.to_owned(), value);
}

pub(crate) fn hook_metadata(auth: Json) -> IndexMap<String, Json> {
    IndexMap::from([("user_api_key_auth".to_owned(), auth)])
}

pub(crate) fn team_row(team_id: &str, max_budget: &str, spend: &str) -> LiteLLMTeamTableCachedObj {
    serde_json::from_str(&format!(
        r#"{{"team_id":"{team_id}","max_budget":{max_budget},"spend":{spend}}}"#
    ))
    .unwrap()
}

pub(crate) fn shadow_reply() -> ShadowResponse {
    ShadowResponse {
        text: "shadow answer".to_owned(),
        model: "shadow-model".to_owned(),
        tier: Some("premium".to_owned()),
        cost: decimal("0.25"),
        classifier_cost: decimal("0.5"),
    }
}

pub(crate) fn judge_verdict(preference: &str) -> JudgeVerdict {
    JudgeVerdict {
        preference: preference.to_owned(),
        confidence: decimal("0.9"),
        cost: Decimal::ONE,
    }
}

pub(crate) fn call_failure(error: &str, cost: &str, classifier_cost: &str) -> CallFailure {
    CallFailure {
        error: error.to_owned(),
        cost: decimal(cost),
        classifier_cost: decimal(classifier_cost),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpendScript {
    Value(Decimal),
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpendRead {
    pub(crate) counter_key: String,
    pub(crate) fallback_spend: Decimal,
    pub(crate) max_budget: Option<Decimal>,
}

pub(crate) struct SpendFake {
    pub(crate) key: SpendScript,
    pub(crate) team: SpendScript,
    reads: Mutex<Vec<SpendRead>>,
}

impl SpendFake {
    pub(crate) fn new(key: SpendScript, team: SpendScript) -> Self {
        Self {
            key,
            team,
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<SpendRead> {
        self.reads.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendReader for SpendFake {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        self.reads.lock().unwrap().push(SpendRead {
            counter_key: query.counter_key.to_owned(),
            fallback_spend: query.fallback_spend,
            max_budget: query.max_budget,
        });
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

pub(crate) enum TeamAnswer {
    Found(Box<LiteLLMTeamTableCachedObj>),
    NotInCache,
    NoDatabase,
}

pub(crate) struct TeamsFake {
    pub(crate) answer: TeamAnswer,
    lookups: Mutex<Vec<String>>,
}

impl TeamsFake {
    pub(crate) fn new(answer: TeamAnswer) -> Self {
        Self {
            answer,
            lookups: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn lookups(&self) -> Vec<String> {
        self.lookups.lock().unwrap().clone()
    }
}

impl CachedTeams for TeamsFake {
    fn cached_team<'lookup>(&'lookup self, team_id: &'lookup str) -> TeamFuture<'lookup> {
        self.lookups.lock().unwrap().push(team_id.to_owned());
        let answer = match &self.answer {
            TeamAnswer::Found(team) => Ok(team.as_ref().clone()),
            TeamAnswer::NotInCache => Err(TeamLookupError::NotInCache {
                team_id: team_id.to_owned(),
            }),
            TeamAnswer::NoDatabase => Err(TeamLookupError::NoDatabase),
        };
        Box::pin(ready(answer))
    }
}

#[derive(Default)]
pub(crate) struct Alerts {
    budget: Mutex<Vec<BudgetAlert>>,
    team: Mutex<Vec<TeamAlert>>,
}

impl Alerts {
    pub(crate) fn budget_alerts(&self) -> Vec<BudgetAlert> {
        self.budget.lock().unwrap().clone()
    }

    pub(crate) fn team_alerts(&self) -> Vec<TeamAlert> {
        self.team.lock().unwrap().clone()
    }
}

impl BudgetAlertSink for Alerts {
    fn schedule(&self, alert: BudgetAlert) {
        self.budget.lock().unwrap().push(alert);
    }
}

impl TeamAlertSink for Alerts {
    fn schedule(&self, alert: TeamAlert) {
        self.team.lock().unwrap().push(alert);
    }
}

pub(crate) fn eval_job(
    id: &str,
    arms: &[&str],
    max_budget: Option<Decimal>,
) -> ActiveShadowEvalJob {
    ActiveShadowEvalJob {
        id: id.to_owned(),
        router_name: arms.first().copied().unwrap_or_default().to_owned(),
        arm_router_names: arms.iter().map(|arm| (*arm).to_owned()).collect(),
        models: BTreeSet::from(["gpt-4o".to_owned()]),
        direction: ShadowEvalDirection::Forward,
        baseline_model: None,
        shadow_percentage: 0.5_f64,
        judge_model: "judge-model".to_owned(),
        max_turns: 3_i64,
        max_budget,
        ends_at: Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap(),
        attempts: 7_i64,
        spend: Decimal::ZERO,
    }
}

#[derive(Default)]
pub(crate) struct SampleFixture {
    pub(crate) request_id: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) real_text: String,
    pub(crate) real_model: String,
    pub(crate) real_cost: Decimal,
    pub(crate) real_classifier_cost: Decimal,
    pub(crate) real_cache_hit: bool,
    pub(crate) control_tier: Option<String>,
    pub(crate) shadow_params: IndexMap<String, Json>,
    pub(crate) parent_metadata: IndexMap<String, Json>,
}

impl SampleFixture {
    pub(crate) fn new() -> Self {
        let mut fixture = Self::default();
        fixture.seed();
        fixture
    }

    fn seed(&mut self) {
        self.request_id = "req-1".to_owned();
        self.messages.push(Json::Object(IndexMap::from([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text("hi".to_owned())),
        ])));
        self.real_text = "real answer".to_owned();
        self.real_model = "gpt-4o".to_owned();
        self.real_cost = decimal("0.1");
        self.real_classifier_cost = decimal("0.02");
        self.real_cache_hit = true;
    }

    pub(crate) fn sample(&self) -> ShadowSample<'_> {
        ShadowSample {
            parent_metadata: &self.parent_metadata,
            shadow_params: &self.shadow_params,
            control_tier: self.control_tier.as_deref(),
            real_cache_hit: self.real_cache_hit,
            real_classifier_cost: self.real_classifier_cost,
            real_cost: self.real_cost,
            real_model: &self.real_model,
            real_text: &self.real_text,
            messages: &self.messages,
            request_id: &self.request_id,
        }
    }
}

pub(crate) struct Attempts {
    rows: Mutex<Vec<ShadowAttemptRow>>,
}

impl Attempts {
    pub(crate) fn new() -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn rows(&self) -> Vec<ShadowAttemptRow> {
        self.rows.lock().unwrap().clone()
    }
}

impl ShadowTables for Attempts {
    type Error = TestError;

    fn find_active_jobs(
        &self,
        _now: DateTime<Utc>,
    ) -> ShadowFuture<'_, Vec<ShadowJobRow>, TestError> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn attempt_stats<'database>(
        &'database self,
        _job_ids: &'database [String],
    ) -> ShadowFuture<'database, Vec<JobAttemptStats>, TestError> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn insert_attempt<'database>(
        &'database self,
        attempt: &'database ShadowAttemptRow,
    ) -> ShadowFuture<'database, (), TestError> {
        let row = attempt.clone();
        Box::pin(async move {
            self.rows.lock().unwrap().push(row);
            Ok(())
        })
    }

    fn upsert_funnel<'database>(
        &'database self,
        _job_id: &'database str,
        _counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), TestError> {
        Box::pin(ready(Ok(())))
    }
}

pub(crate) struct SpendLog {
    entries: Mutex<Vec<(String, Decimal)>>,
}

impl SpendLog {
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn entries(&self) -> Vec<(String, Decimal)> {
        self.entries.lock().unwrap().clone()
    }
}

impl SpendCounter for SpendLog {
    type Error = TestError;

    fn add_spend<'counter>(
        &'counter self,
        counter_key: &'counter str,
        cost: Decimal,
    ) -> SpendFuture<'counter, (), TestError> {
        let key = counter_key.to_owned();
        Box::pin(async move {
            self.entries.lock().unwrap().push((key, cost));
            Ok(())
        })
    }
}

pub(crate) struct FunnelLog {
    events: Mutex<Vec<(String, FunnelStage)>>,
}

impl FunnelLog {
    pub(crate) fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn events(&self) -> Vec<(String, FunnelStage)> {
        self.events.lock().unwrap().clone()
    }
}

impl FunnelRecorder for FunnelLog {
    fn record(&self, job_id: &str, stage: FunnelStage) {
        self.events.lock().unwrap().push((job_id.to_owned(), stage));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpendAnswer {
    Spend(Decimal),
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JobSpendRead {
    pub(crate) counter_key: String,
    pub(crate) fallback_spend: Decimal,
    pub(crate) max_budget: Decimal,
}

pub(crate) struct JobSpendFake {
    pub(crate) answer: SpendAnswer,
    reads: Mutex<Vec<JobSpendRead>>,
}

impl JobSpendFake {
    pub(crate) fn new(answer: SpendAnswer) -> Self {
        Self {
            answer,
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<JobSpendRead> {
        self.reads.lock().unwrap().clone()
    }
}

impl JobSpendReader for JobSpendFake {
    fn read_spend<'reader>(
        &'reader self,
        counter_key: &'reader str,
        fallback_spend: Decimal,
        max_budget: Decimal,
    ) -> JobSpendFuture<'reader> {
        self.reads.lock().unwrap().push(JobSpendRead {
            counter_key: counter_key.to_owned(),
            fallback_spend,
            max_budget,
        });
        let answer = match self.answer {
            SpendAnswer::Spend(spend) => Ok(spend),
            SpendAnswer::Unreadable => Err(SpendReadError::Unverifiable {
                counter_key: counter_key.to_owned(),
            }),
        };
        Box::pin(ready(answer))
    }
}

#[derive(Debug)]
pub(crate) enum Answer<Value> {
    Reply(Value),
    Failed(CallFailure),
    Raised(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShadowCallLog {
    pub(crate) target_model: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) shadow_params: IndexMap<String, Json>,
}

impl ShadowCallLog {
    fn of(request: &ShadowRequest<'_>) -> Self {
        Self {
            target_model: request.target_model.to_owned(),
            messages: request.messages.to_vec(),
            shadow_params: request.shadow_params.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JudgeCallLog {
    pub(crate) judge_model: String,
    pub(crate) real_text: String,
    pub(crate) shadow_text: String,
    pub(crate) tools: Json,
}

impl JudgeCallLog {
    fn of(request: &JudgeRequest<'_>) -> Self {
        Self {
            judge_model: request.judge_model.to_owned(),
            real_text: request.real_text.to_owned(),
            shadow_text: request.shadow_text.to_owned(),
            tools: request.tools.clone(),
        }
    }
}

pub(crate) struct CallsFake {
    shadow_answers: Mutex<VecDeque<Answer<ShadowResponse>>>,
    judge_answers: Mutex<VecDeque<Answer<JudgeVerdict>>>,
    shadow_calls: Mutex<Vec<ShadowCallLog>>,
    judge_calls: Mutex<Vec<JudgeCallLog>>,
}

impl CallsFake {
    pub(crate) fn new(
        shadow: Vec<Answer<ShadowResponse>>,
        judge: Vec<Answer<JudgeVerdict>>,
    ) -> Self {
        Self {
            shadow_answers: Mutex::new(VecDeque::from(shadow)),
            judge_answers: Mutex::new(VecDeque::from(judge)),
            shadow_calls: Mutex::new(Vec::new()),
            judge_calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn shadow_calls(&self) -> Vec<ShadowCallLog> {
        self.shadow_calls.lock().unwrap().clone()
    }

    pub(crate) fn judge_calls(&self) -> Vec<JudgeCallLog> {
        self.judge_calls.lock().unwrap().clone()
    }

    pub(crate) fn shadow_targets(&self) -> Vec<String> {
        self.shadow_calls
            .lock()
            .unwrap()
            .iter()
            .map(|call| call.target_model.clone())
            .collect()
    }
}

impl CallsFake {
    fn next<Value>(answers: &Mutex<VecDeque<Answer<Value>>>, stage: &str) -> ArmStage<Value> {
        match answers.lock().unwrap().pop_front() {
            Some(Answer::Reply(value)) => Ok(value),
            Some(Answer::Failed(failure)) => Err(ArmFailure::Failed(failure)),
            Some(Answer::Raised(message)) => Err(ArmFailure::Raised(message.to_owned())),
            None => panic!("the fake ran out of {stage} answers"),
        }
    }
}

#[async_trait]
impl ArmCalls for CallsFake {
    async fn shadow(&self, request: &ShadowRequest<'_>) -> ArmStage<ShadowResponse> {
        self.shadow_calls
            .lock()
            .unwrap()
            .push(ShadowCallLog::of(request));
        Self::next(&self.shadow_answers, "shadow")
    }

    async fn judge(&self, request: &JudgeRequest<'_>) -> ArmStage<JudgeVerdict> {
        self.judge_calls
            .lock()
            .unwrap()
            .push(JudgeCallLog::of(request));
        Self::next(&self.judge_answers, "judge")
    }
}
