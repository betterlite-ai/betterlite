use std::collections::BTreeMap;
use std::sync::Mutex;

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
    CallsFake, JobSpendFake, Recorder, SpendFake, SpendScript, TeamsFake, build_runtime, failure,
    locked,
};

const JOB_ID: &str = "job-1";
const REQUEST_ID: &str = "req-1";
const KEY_HASH: &str = "key-hash";
const JUDGE_MODEL: &str = "judge-model";
const REAL_MODEL: &str = "claude-opus";
const REAL_TEXT: &str = "real answer";

pub(crate) fn pipeline(call: &PipelineCall) -> HelperResult {
    let job = active_job(call)?;
    let metadata = metadata(call)?;
    let recorder = Recorder::default();
    let spend = SpendFake {
        key: script(call.key_spend),
        team: script(call.team_spend),
        reads: Mutex::new(Vec::new()),
    };
    let teams = TeamsFake {
        team: call.team.as_ref().map(team_row).transpose()?,
    };
    let job_spend = JobSpendFake {
        spend: call.job_spend.unwrap_or(Decimal::ZERO),
        reads: Mutex::new(Vec::new()),
    };
    let calls = CallsFake::new(call);
    let advisory = AdvisoryDeps {
        spend: &spend,
        teams: &teams,
        key_alerts: None,
        team_alerts: None,
        throttle_pct: call.litellm_settings.throttle_pct,
        now: DateTime::<Utc>::UNIX_EPOCH,
    };
    let deps = PipelineDeps {
        database: call.database.then_some(&recorder),
        counter: &recorder,
        funnel: &recorder,
        advisory: &advisory,
        spend: &job_spend,
        calls: &calls,
    };
    let messages = shadow_messages();
    let shadow_params = IndexMap::new();
    let sample = shadow_sample(&messages, &shadow_params, &metadata);
    build_runtime()?.block_on(run_shadow_eval(&job, &sample, &deps));
    Ok(output(&recorder, &spend, &job_spend))
}

fn shadow_messages() -> [Json; 1] {
    [Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::Text("what is 2+2?".to_owned())),
    ]))]
}

fn shadow_sample<'sample>(
    messages: &'sample [Json],
    shadow_params: &'sample IndexMap<String, Json>,
    parent_metadata: &'sample IndexMap<String, Json>,
) -> ShadowSample<'sample> {
    ShadowSample {
        request_id: REQUEST_ID,
        messages,
        real_text: REAL_TEXT,
        real_model: REAL_MODEL,
        real_cost: Decimal::new(25, 2),
        real_classifier_cost: Decimal::ZERO,
        real_cache_hit: false,
        control_tier: None,
        shadow_params,
        parent_metadata,
    }
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
    let mut router_names = routers;
    if router_names.len() < 2 {
        router_names.clear();
    }
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
    let mut fields = IndexMap::from([(
        "user_api_key_hash".to_owned(),
        Json::Text(KEY_HASH.to_owned()),
    )]);
    if let Some(auth) = &call.auth {
        let mut record = IndexMap::from([
            ("token".to_owned(), Json::Text(auth.token.clone())),
            ("spend".to_owned(), Json::Number(number(auth.spend)?)),
        ]);
        if let Some(max_budget) = auth.max_budget {
            record.insert("max_budget".to_owned(), Json::Number(number(max_budget)?));
        }
        if let Some(team_id) = &auth.team_id {
            record.insert("team_id".to_owned(), Json::Text(team_id.clone()));
        }
        if let Some(fields) = &auth.metadata {
            record.insert("metadata".to_owned(), Json::Object(fields.clone()));
        }
        if let Some(tpm_limit) = auth.tpm_limit {
            record.insert(
                "tpm_limit".to_owned(),
                Json::Number(Number::from(tpm_limit)),
            );
        }
        if let Some(rpm_limit) = auth.rpm_limit {
            record.insert(
                "rpm_limit".to_owned(),
                Json::Number(Number::from(rpm_limit)),
            );
        }
        fields.insert("user_api_key_auth".to_owned(), Json::Object(record));
    }
    Ok(fields)
}

// the auth and team readers take a JSON number exactly as LiteLLM's pydantic payloads carry it
fn number(value: Decimal) -> Result<Number, Box<HelperError>> {
    value
        .normalize()
        .to_string()
        .parse::<Number>()
        .map_err(|error| failure("ValueError", &error.to_string()))
}

fn team_row(team: &TeamInput) -> Result<LiteLLMTeamTableCachedObj, Box<HelperError>> {
    let maximum = JsonData::Number(number(team.max_budget)?);
    let row = BTreeMap::from([
        ("team_id".to_owned(), JsonData::String(team.team_id.clone())),
        ("max_budget".to_owned(), maximum),
        ("spend".to_owned(), JsonData::Number(number(team.spend)?)),
    ]);
    LiteLLMTeamTableCachedObj::try_from(JsonData::Object(row))
        .map_err(|error| failure("ValueError", &error.to_string()))
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

fn output(recorder: &Recorder, spend: &SpendFake, job_spend: &JobSpendFake) -> Value {
    let funnel_events: Vec<Value> = locked(&recorder.events)
        .iter()
        .map(|(job_id, stage)| json!([job_id, stage_name(*stage)]))
        .collect();
    let rows: Vec<Value> = locked(&recorder.rows).iter().map(row_json).collect();
    let writes = locked(&recorder.writes).clone();
    let advisory_reads = locked(&spend.reads).clone();
    let job_spend_reads = locked(&job_spend.reads).clone();
    json!({
        "funnel": funnel_events,
        "rows": rows,
        "spend_writes": writes,
        "advisory_reads": advisory_reads,
        "job_spend_reads": job_spend_reads,
    })
}
