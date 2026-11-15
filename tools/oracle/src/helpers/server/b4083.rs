use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::shadow_tables::rows::{
    AttemptOutcome, FunnelCounters, JobAttemptStats, ShadowAttemptRow, ShadowFuture, ShadowJobRow,
    ShadowTables,
};
use betterlite_server::shadow_eval_job_loading::attempt::{
    SpendCounter, SpendFuture, record_attempt,
};
use betterlite_server::shadow_eval_job_loading::cache::Clock;
use betterlite_server::shadow_eval_job_loading::jobs::ActiveShadowEvalJob;
use betterlite_server::shadow_eval_job_loading::loader::{LoadedJobs, ShadowJobLoader};
use betterlite_server::shadow_eval_job_loading::sampling::{RequestAdmission, sampled_jobs};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ValidateJob(ValidateCall),
    LoadJobs(LoadCall),
    RecordAttempt(AttemptCall),
}

#[derive(Deserialize)]
struct ValidateCall {
    arm_router: String,
    rows: Vec<JobInput>,
}

#[derive(Deserialize)]
struct LoadCall {
    rows: Vec<JobInput>,
    #[serde(default)]
    stats: Vec<StatInput>,
    #[serde(default)]
    metadata: BTreeMap<String, Value>,
    model_group: String,
    #[serde(default)]
    aliases: BTreeMap<String, String>,
    #[serde(default)]
    hook_calls: usize,
}

#[derive(Deserialize)]
struct AttemptCall {
    job: JobInput,
    row: RowInput,
}

#[derive(Deserialize)]
struct JobInput {
    id: String,
    target_type: String,
    target_id: String,
    #[serde(default)]
    stopped_at: Option<DateTime<Utc>>,
    router_name: String,
    #[serde(default)]
    router_names: Vec<String>,
    #[serde(default)]
    models: Vec<String>,
    direction: String,
    #[serde(default)]
    baseline_model: Option<String>,
    shadow_percentage: f64,
    judge_model: String,
    max_turns: i64,
    #[serde(default)]
    max_budget: Option<Decimal>,
    ends_at: DateTime<Utc>,
}

impl JobInput {
    fn to_row(&self) -> ShadowJobRow {
        ShadowJobRow {
            id: self.id.clone(),
            target_type: self.target_type.clone(),
            target_id: self.target_id.clone(),
            router_name: self.router_name.clone(),
            router_names: self.router_names.clone(),
            models: self.models.clone(),
            direction: self.direction.clone(),
            baseline_model: self.baseline_model.clone(),
            shadow_percentage: self.shadow_percentage,
            judge_model: self.judge_model.clone(),
            max_turns: self.max_turns,
            max_budget: self.max_budget,
            ends_at: self.ends_at,
        }
    }
}

#[derive(Deserialize)]
struct StatInput {
    job_id: String,
    attempts: i64,
    spend: Decimal,
}

#[derive(Deserialize)]
struct RowInput {
    request_id: String,
    router_name: String,
    outcome: String,
    #[serde(default)]
    tier: Option<String>,
    real_model: String,
    #[serde(default)]
    shadow_model: Option<String>,
    #[serde(default)]
    confidence: Option<Decimal>,
    judge_cost: Decimal,
    shadow_cost: Decimal,
    shadow_classifier_cost: Decimal,
    real_cost: Decimal,
    real_classifier_cost: Decimal,
    real_cache_hit: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("scripted database failure")]
struct Scripted;

struct StoredJob {
    stopped_at: Option<DateTime<Utc>>,
    row: ShadowJobRow,
}

struct LoadTables {
    rows: Vec<StoredJob>,
    stats: Vec<JobAttemptStats>,
    find_calls: Mutex<usize>,
    stats_requests: Mutex<Vec<String>>,
}

impl LoadTables {
    fn new(rows: &[JobInput], stats: &[StatInput]) -> Self {
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
            find_calls: Mutex::new(0),
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
            let mut find_calls = self
                .find_calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *find_calls = find_calls.saturating_add(1);
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

struct AttemptTables {
    events: Arc<Mutex<Vec<String>>>,
    inserts: Mutex<Vec<ShadowAttemptRow>>,
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
        Box::pin(async move {
            self.events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("insert:{}", attempt.request_id));
            self.inserts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ShadowAttemptRow::clone(attempt));
            Ok(())
        })
    }

    fn upsert_funnel<'database>(
        &'database self,
        _job_id: &'database str,
        _counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async { Ok(()) })
    }
}

struct FakeCounter {
    events: Arc<Mutex<Vec<String>>>,
}

impl SpendCounter for FakeCounter {
    type Error = Scripted;

    fn read_spend<'counter>(
        &'counter self,
        _counter_key: &'counter str,
        fallback_spend: Decimal,
        _max_budget: Decimal,
    ) -> SpendFuture<'counter, Decimal, Self::Error> {
        Box::pin(async move { Ok(fallback_spend) })
    }

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

fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
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

fn candidates<'jobs>(
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

fn validate_row(input: &JobInput, arm_router: &str) -> Value {
    let Ok(job) = ActiveShadowEvalJob::from_row(&input.to_row(), 0, Decimal::ZERO) else {
        return json!({
            "id": input.id,
            "valid": false,
            "direction": Value::Null,
            "arm_router_names": Vec::<String>::new(),
            "arm_target": Value::Null,
        });
    };
    json!({
        "id": job.id,
        "valid": true,
        "direction": job.direction.as_str(),
        "arm_router_names": job.arm_router_names,
        "arm_target": job.arm_target(arm_router),
    })
}

fn validate(call: &ValidateCall) -> Value {
    let jobs: Vec<Value> = call
        .rows
        .iter()
        .map(|input| validate_row(input, &call.arm_router))
        .collect();
    json!({"jobs": jobs})
}

fn load(call: &LoadCall, runtime: &tokio::runtime::Runtime) -> Result<Value, Box<HelperError>> {
    let tables = Arc::new(LoadTables::new(&call.rows, &call.stats));
    let spend_writes = Arc::new(Mutex::new(Vec::new()));
    let counter = FakeCounter {
        events: Arc::clone(&spend_writes),
    };
    let clock: Clock = Arc::new(|| 0.0_f64);
    let loader = ShadowJobLoader::new(Some(Arc::clone(&tables)), counter, clock)
        .map_err(|error| failure("RuntimeError", error))?;
    let now = "2030-01-01T00:00:00Z"
        .parse::<DateTime<Utc>>()
        .map_err(|error| failure("InvalidInput", error))?;
    let funnel = runtime.block_on(async {
        let mut starts: BTreeMap<String, i64> = BTreeMap::new();
        let mut funnel: Vec<Value> = Vec::new();
        for _hook in 0..call.hook_calls {
            let loaded = loader.active_jobs(now).await;
            if loaded.refilled {
                starts.clear();
            }
            let candidates = candidates(&loaded, &call.metadata);
            let routed_by = call
                .metadata
                .get("routing_decision")
                .and_then(Value::as_object)
                .and_then(|decision| decision.get("router_model_name"))
                .and_then(Value::as_str);
            let request = RequestAdmission {
                routed_by,
                model_group: &call.model_group,
                request_id: "req-1",
            };
            let sampled = sampled_jobs(&candidates, &request, &starts, &call.aliases, now);
            for job_id in &sampled.not_sampled {
                funnel.push(json!([job_id, "not_sampled"]));
            }
            for job in &sampled.eligible {
                funnel.push(json!([job.id, "unjudgeable"]));
            }
        }
        funnel
    });
    Ok(json!({
        "find_calls": *tables.find_calls.lock().unwrap_or_else(PoisonError::into_inner),
        "group_by": ["job_id"],
        "group_by_count": true,
        "group_by_sum": ["judge_cost", "shadow_classifier_cost", "shadow_cost"],
        "stats_ids": tables
            .stats_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .unwrap_or_default(),
        "funnel": funnel,
        "attempts": 0,
        "spend_writes": spend_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    }))
}

fn outcome(raw: &str) -> Result<AttemptOutcome, Box<HelperError>> {
    match raw {
        "real" => Ok(AttemptOutcome::Real),
        "shadow" => Ok(AttemptOutcome::Shadow),
        "tie" => Ok(AttemptOutcome::Tie),
        "error" => Ok(AttemptOutcome::Error),
        other => Err(failure("InvalidInput", format!("unknown outcome {other}"))),
    }
}

fn record(
    call: &AttemptCall,
    runtime: &tokio::runtime::Runtime,
) -> Result<Value, Box<HelperError>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let tables = AttemptTables {
        events: Arc::clone(&events),
        inserts: Mutex::new(Vec::new()),
    };
    let counter = FakeCounter {
        events: Arc::clone(&events),
    };
    let attempt = ShadowAttemptRow {
        job_id: call.job.id.clone(),
        request_id: call.row.request_id.clone(),
        router_name: call.row.router_name.clone(),
        outcome: outcome(&call.row.outcome)?,
        tier: call.row.tier.clone(),
        real_model: Some(call.row.real_model.clone()),
        shadow_model: call.row.shadow_model.clone(),
        confidence: call.row.confidence,
        judge_cost: call.row.judge_cost,
        shadow_cost: call.row.shadow_cost,
        shadow_classifier_cost: call.row.shadow_classifier_cost,
        real_cost: Some(call.row.real_cost),
        real_classifier_cost: call.row.real_classifier_cost,
        real_cache_hit: call.row.real_cache_hit,
        error: call.row.error.clone(),
    };
    runtime.block_on(record_attempt(Some(&tables), &counter, attempt));
    let stored = tables
        .inserts
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned();
    let Some(stored) = stored else {
        return Err(failure("RuntimeError", "no attempt row was inserted"));
    };
    let row = json!({
        "request_id": stored.request_id,
        "outcome": stored.outcome.as_str(),
        "tier": stored.tier,
        "real_model": stored.real_model,
        "shadow_model": stored.shadow_model,
        "confidence": stored.confidence.map(|value| value.normalize().to_string()),
        "judge_cost": stored.judge_cost.normalize().to_string(),
        "shadow_cost": stored.shadow_cost.normalize().to_string(),
        "shadow_classifier_cost": stored.shadow_classifier_cost.normalize().to_string(),
        "real_cost": stored.real_cost.map(|value| value.normalize().to_string()),
        "real_classifier_cost": stored.real_classifier_cost.normalize().to_string(),
        "real_cache_hit": stored.real_cache_hit,
        "error_chars": stored.error.as_ref().map_or(0, |error| error.chars().count()),
    });
    Ok(json!({
        "events": events.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        "row": row,
        "funnel": Vec::<Value>::new(),
    }))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ValidateJob(call) => Ok(validate(call)),
            Input::LoadJobs(call) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .map_err(|error| failure("RuntimeError", error))?;
                load(call, &runtime)
            }
            Input::RecordAttempt(call) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .map_err(|error| failure("RuntimeError", error))?;
                record(call, &runtime)
            }
        }
    }
}
