use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::shadow_tables::rows::{FunnelStage, ShadowAttemptRow, ShadowJobRow};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_server::shadow_eval_job_loading::jobs::ActiveShadowEvalJob;
use betterlite_server::shadow_evaluation::ShadowSample;
use betterlite_server::shadow_evaluation::advisory_budget::AdvisoryDeps;
use betterlite_server::shadow_evaluation::pipeline::{PipelineDeps, run_shadow_eval};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Number, Value, json};

use super::inputs::{PipelineCall, TeamInput};
use super::support::{
    Attempts, CallsFake, FunnelLog, JobSpendFake, SpendFake, SpendLog, SpendScript, TeamsFake,
    build_runtime, failure,
};

const JOB_ID: &str = "job-1";
const REQUEST_ID: &str = "req-1";
const KEY_HASH: &str = "key-hash";
const JUDGE_MODEL: &str = "judge-model";
const REAL_MODEL: &str = "claude-opus";
const REAL_TEXT: &str = "real answer";

pub(crate) fn pipeline(call: &PipelineCall) -> HelperResult {
    let job = active_job(call)?;
    let fixture = Fixture::new(metadata(call)?);
    let attempts = Attempts::default();
    let counter = SpendLog::default();
    let funnel = FunnelLog::default();
    let spend = SpendFake::new(script(call.key_spend), script(call.team_spend));
    let teams = TeamsFake::new(call.team.as_ref().map(team_row).transpose()?);
    let job_spend = JobSpendFake::new(call.job_spend.unwrap_or(Decimal::ZERO));
    let calls = CallsFake::new(call);
    let advisory = AdvisoryDeps {
        spend: &spend,
        teams: &teams,
        key_alerts: None,
        team_alerts: None,
        now: DateTime::<Utc>::UNIX_EPOCH,
    };
    let deps = PipelineDeps {
        database: call.database.then_some(&attempts),
        counter: &counter,
        funnel: &funnel,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    let runtime = build_runtime()?;
    runtime.block_on(run_shadow_eval(&job, &fixture.sample(), &deps));
    Ok(output(&funnel, &attempts, &counter, &spend, &job_spend))
}

const fn script(spend: Option<Decimal>) -> SpendScript {
    match spend {
        Some(value) => SpendScript::Value(value),
        None => SpendScript::Unreadable,
    }
}

fn active_job(call: &PipelineCall) -> Result<ActiveShadowEvalJob, Box<HelperError>> {
    let routers: Vec<String> = call.arms.iter().map(|arm| arm.router.clone()).collect();
    let Some(first) = routers.first().cloned() else {
        return Err(failure("ValueError", "the job needs at least one arm"));
    };
    let router_names = if routers.len() > 1 {
        routers
    } else {
        Vec::new()
    };
    let row = ShadowJobRow {
        id: JOB_ID.to_owned(),
        target_type: "key".to_owned(),
        target_id: KEY_HASH.to_owned(),
        router_name: first,
        router_names,
        models: Vec::new(),
        direction: "forward".to_owned(),
        baseline_model: None,
        shadow_percentage: 100.0_f64,
        judge_model: JUDGE_MODEL.to_owned(),
        max_turns: 5_i64,
        max_budget: call.max_budget,
        ends_at: DateTime::<Utc>::UNIX_EPOCH,
    };
    ActiveShadowEvalJob::from_row(&row, 0_i64, call.stats_spend.unwrap_or(Decimal::ZERO))
        .map_err(|error| failure("ValueError", &error.to_string()))
}

fn metadata(call: &PipelineCall) -> Result<IndexMap<String, Json>, Box<HelperError>> {
    let mut fields = IndexMap::new();
    fields.insert(
        "user_api_key_hash".to_owned(),
        Json::Text(KEY_HASH.to_owned()),
    );
    if let Some(auth) = &call.auth {
        let mut record = IndexMap::new();
        record.insert("token".to_owned(), Json::Text(auth.token.clone()));
        record.insert("spend".to_owned(), json_number(auth.spend)?);
        if let Some(max_budget) = auth.max_budget {
            record.insert("max_budget".to_owned(), json_number(max_budget)?);
        }
        if let Some(team_id) = &auth.team_id {
            record.insert("team_id".to_owned(), Json::Text(team_id.clone()));
        }
        fields.insert("user_api_key_auth".to_owned(), Json::Object(record));
    }
    Ok(fields)
}

fn json_number(value: Decimal) -> Result<Json, Box<HelperError>> {
    Ok(Json::Number(number(value)?))
}

fn data_number(value: Decimal) -> Result<JsonData, Box<HelperError>> {
    Ok(JsonData::Number(number(value)?))
}

// the auth and team readers take a JSON number exactly as LiteLLM's pydantic payloads carry it
fn number(value: Decimal) -> Result<Number, Box<HelperError>> {
    value
        .normalize()
        .to_string()
        .parse()
        .map_err(|error| failure("ValueError", &error.to_string()))
}

fn team_row(team: &TeamInput) -> Result<LiteLLMTeamTableCachedObj, Box<HelperError>> {
    let row = BTreeMap::from([
        ("team_id".to_owned(), JsonData::String(team.team_id.clone())),
        ("max_budget".to_owned(), data_number(team.max_budget)?),
        ("spend".to_owned(), data_number(team.spend)?),
    ]);
    LiteLLMTeamTableCachedObj::try_from(JsonData::Object(row))
        .map_err(|error| failure("ValueError", &error.to_string()))
}

struct Fixture {
    messages: Vec<Json>,
    real_text: String,
    real_model: String,
    real_cost: Decimal,
    real_classifier_cost: Decimal,
    real_cache_hit: bool,
    shadow_params: IndexMap<String, Json>,
    parent_metadata: IndexMap<String, Json>,
}

impl Fixture {
    fn new(parent_metadata: IndexMap<String, Json>) -> Self {
        Self {
            messages: vec![Json::Object(IndexMap::from([
                ("role".to_owned(), Json::Text("user".to_owned())),
                ("content".to_owned(), Json::Text("what is 2+2?".to_owned())),
            ]))],
            real_text: REAL_TEXT.to_owned(),
            real_model: REAL_MODEL.to_owned(),
            real_cost: Decimal::new(25, 2),
            real_classifier_cost: Decimal::ZERO,
            real_cache_hit: false,
            shadow_params: IndexMap::new(),
            parent_metadata,
        }
    }

    fn sample(&self) -> ShadowSample<'_> {
        ShadowSample {
            request_id: REQUEST_ID,
            messages: &self.messages,
            real_text: &self.real_text,
            real_model: &self.real_model,
            real_cost: self.real_cost,
            real_classifier_cost: self.real_classifier_cost,
            real_cache_hit: self.real_cache_hit,
            control_tier: None,
            shadow_params: &self.shadow_params,
            parent_metadata: &self.parent_metadata,
        }
    }
}

fn row_json(row: &ShadowAttemptRow) -> Value {
    json!({
        "job_id": row.job_id,
        "request_id": row.request_id,
        "router_name": row.router_name,
        "outcome": row.outcome.as_str(),
        "tier": row.tier,
        "real_model": row.real_model,
        "shadow_model": row.shadow_model,
        "confidence": row.confidence.map(|value| value.normalize().to_string()),
        "judge_cost": row.judge_cost.normalize().to_string(),
        "shadow_cost": row.shadow_cost.normalize().to_string(),
        "shadow_classifier_cost": row.shadow_classifier_cost.normalize().to_string(),
        "real_cost": row.real_cost.map(|value| value.normalize().to_string()),
        "real_classifier_cost": row.real_classifier_cost.normalize().to_string(),
        "real_cache_hit": row.real_cache_hit,
        "error_present": row.error.is_some(),
    })
}

const fn stage_name(stage: FunnelStage) -> &'static str {
    match stage {
        FunnelStage::NotSampled => "not_sampled",
        FunnelStage::Unjudgeable => "unjudgeable",
        FunnelStage::Shed => "shed",
        FunnelStage::Withheld => "withheld",
    }
}

fn output(
    funnel: &FunnelLog,
    attempts: &Attempts,
    counter: &SpendLog,
    spend: &SpendFake,
    job_spend: &JobSpendFake,
) -> Value {
    json!({
        "funnel": funnel_json(funnel),
        "rows": rows_json(attempts),
        "spend_writes": writes_json(counter),
        "advisory_reads": advisory_reads_json(spend),
        "job_spend_reads": job_spend_reads_json(job_spend),
    })
}

fn funnel_json(funnel: &FunnelLog) -> Value {
    let events = funnel.events.lock().unwrap_or_else(PoisonError::into_inner);
    let mut pairs = Vec::new();
    for (job_id, stage) in events.iter() {
        pairs.push(json!([job_id, stage_name(*stage)]));
    }
    json!(pairs)
}

fn rows_json(attempts: &Attempts) -> Value {
    let rows = attempts.rows.lock().unwrap_or_else(PoisonError::into_inner);
    let mut values = Vec::new();
    for row in rows.iter() {
        values.push(row_json(row));
    }
    json!(values)
}

fn writes_json(counter: &SpendLog) -> Value {
    let writes = counter
        .writes
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut values = Vec::new();
    for write in writes.iter() {
        values.push(json!(write));
    }
    json!(values)
}

fn advisory_reads_json(spend: &SpendFake) -> Value {
    let reads = spend.reads.lock().unwrap_or_else(PoisonError::into_inner);
    let mut values = Vec::new();
    for read in reads.iter() {
        values.push(json!([
            read.counter_key,
            read.fallback_spend.normalize().to_string(),
            read.max_budget.map(|value| value.normalize().to_string()),
        ]));
    }
    json!(values)
}

fn job_spend_reads_json(job_spend: &JobSpendFake) -> Value {
    let reads = job_spend
        .reads
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut values = Vec::new();
    for read in reads.iter() {
        values.push(json!([
            read.counter_key,
            read.fallback_spend.normalize().to_string(),
            read.max_budget.normalize().to_string(),
        ]));
    }
    json!(values)
}
