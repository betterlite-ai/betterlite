use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_server::shadow_eval_job_loading::cache::Clock;
use betterlite_server::shadow_eval_job_loading::loader::ShadowJobLoader;
use betterlite_server::shadow_eval_job_loading::sampling::{RequestAdmission, sampled_jobs};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::helpers::HelperResult;

use super::inputs::LoadCall;
use super::support::{FakeCounter, LoadTables, build_runtime, candidates, failure};

pub(crate) fn load(call: &LoadCall) -> HelperResult {
    let runtime = build_runtime()?;
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
    let funnel = runtime.block_on(run_hooks(call, &loader, now));
    let stats_ids = tables
        .stats_requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned()
        .unwrap_or_default();
    Ok(json!({
        "find_calls": *tables.find_calls.lock().unwrap_or_else(PoisonError::into_inner),
        "group_by": ["job_id"],
        "group_by_count": true,
        "group_by_sum": ["judge_cost", "shadow_classifier_cost", "shadow_cost"],
        "stats_ids": stats_ids,
        "funnel": funnel,
        "attempts": 0,
        "spend_writes": spend_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    }))
}

async fn run_hooks(
    call: &LoadCall,
    loader: &ShadowJobLoader<LoadTables, FakeCounter>,
    now: DateTime<Utc>,
) -> Vec<Value> {
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
}
