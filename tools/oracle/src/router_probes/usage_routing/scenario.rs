use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_predicates::JsonMap;
use betterlite_router::usage_routing::record::{SuccessUse, record_success};
use betterlite_router::usage_routing::select::select;
use betterlite_router::usage_routing::window::{Counts, UsageWindows, WindowError, window_keys};
use betterlite_server::routing::usage_routing::resolve;
use chrono::{DateTime, Utc};
use serde_json::Number;

use crate::router_probes::usage_routing::wire::{
    Entry, Input, Observation, Output, Step, StepKind,
};

// The failure a step reports: an instant, a limit or a pool entry the operation cannot read.
#[derive(Debug, thiserror::Error)]
pub(super) enum ScenarioError {
    #[error("{message}")]
    Invalid { message: String },
}

// The window store log_success_event writes and get_available_deployments reads, with gate steps injecting failures.
#[derive(Debug, Default)]
struct Windows {
    counts: Mutex<BTreeMap<String, Counts>>,
    ttls: Mutex<BTreeMap<String, i64>>,
    fail_reads: AtomicBool,
    fail_writes: AtomicBool,
}

impl Windows {
    fn gate(&self, step: &Step) {
        self.fail_reads
            .store(step.fail_reads.unwrap_or(false), Ordering::Relaxed);
        self.fail_writes
            .store(step.fail_writes.unwrap_or(false), Ordering::Relaxed);
    }
}

impl UsageWindows for Windows {
    fn read(&self, key: &str) -> Result<Option<Counts>, WindowError> {
        if self.fail_reads.load(Ordering::Relaxed) {
            return Err(WindowError::Unavailable);
        }
        let Ok(counts) = self.counts.lock() else {
            return Err(WindowError::Unavailable);
        };
        Ok(counts.get(key).cloned())
    }

    fn write(&self, key: &str, counts: &Counts, ttl_seconds: i64) -> Result<(), WindowError> {
        if self.fail_writes.load(Ordering::Relaxed) {
            return Err(WindowError::Unavailable);
        }
        let Ok(mut windows) = self.counts.lock() else {
            return Err(WindowError::Unavailable);
        };
        windows.insert(key.to_owned(), counts.clone());
        drop(windows);
        let Ok(mut ttls) = self.ttls.lock() else {
            return Err(WindowError::Unavailable);
        };
        ttls.insert(key.to_owned(), ttl_seconds);
        Ok(())
    }
}

pub(super) fn run_scenario(input: &Input) -> Result<Output, ScenarioError> {
    let ttl_seconds = resolve_ttl(input.ttl)?;
    let deployments = input
        .deployments
        .iter()
        .map(|entry| deployment(&input.group, entry))
        .collect::<Result<Vec<_>, _>>()?;
    let windows = Windows::default();
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(apply(&windows, input, &deployments, ttl_seconds, step)?);
    }
    Ok(Output { steps })
}

// the configured routing_strategy_args.ttl goes through the production config section and startup resolver
fn resolve_ttl(ttl: Option<i64>) -> Result<i64, ScenarioError> {
    let source = ttl.map_or_else(
        || "router_settings: {}\n".to_owned(),
        |value| {
            format!(
                "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: {value}\n"
            )
        }
    );
    let lookup = BTreeMap::<String, String>::new();
    let config = ConfigLoader { lookup }.load(&source).map_err(invalid)?;
    Ok(resolve(&config.sections.usage_routing).ttl_seconds)
}

fn deployment(group: &str, entry: &Entry) -> Result<DeploymentSpec, ScenarioError> {
    let mut params = JsonObject::from_iter([(
        "model".to_owned(),
        Json::Text(format!("openai/{}", entry.id)),
    )]);
    if let Some(value) = entry.params_tpm {
        params.insert("tpm".to_owned(), Json::Number(Number::from(value)));
    }
    if let Some(value) = entry.params_rpm {
        params.insert("rpm".to_owned(), Json::Number(Number::from(value)));
    }
    let mut info = JsonObject::from_iter([("id".to_owned(), Json::Text(entry.id.clone()))]);
    if let Some(value) = entry.info_tpm {
        info.insert("tpm".to_owned(), Json::Number(Number::from(value)));
    }
    if let Some(value) = entry.info_rpm {
        info.insert("rpm".to_owned(), Json::Number(Number::from(value)));
    }
    let mut extra = JsonObject::new();
    if let Some(value) = entry.tpm {
        extra.insert("tpm".to_owned(), Json::Number(Number::from(value)));
    }
    if let Some(value) = entry.rpm {
        extra.insert("rpm".to_owned(), Json::Number(Number::from(value)));
    }
    DeploymentSpec::new(group.to_owned(), params, info, extra, String::new).map_err(invalid)
}

fn apply(
    windows: &Windows,
    input: &Input,
    deployments: &[DeploymentSpec],
    ttl_seconds: i64,
    step: &Step,
) -> Result<Observation, ScenarioError> {
    let now = timestamp(step.now_millis)?;
    match step.kind {
        // log_success_event and async_log_success_event run the same read-modify-write
        StepKind::Success | StepKind::AsyncSuccess => {
            let params = success_params(&input.group, step);
            let success = SuccessUse {
                call_type: step.call_type.as_deref(),
                litellm_params: &params,
                total_tokens: step.total_tokens,
            };
            record_success(windows, ttl_seconds, now, &success);
            Ok(observed(windows, input, now))
        }
        StepKind::Select => Ok(Observation::Selected {
            id: selected(windows, input, deployments, step, now)?,
        }),
        StepKind::Observe => Ok(observed(windows, input, now)),
        StepKind::Gate => {
            windows.gate(step);
            Ok(Observation::Gated)
        }
    }
}

// the metadata and model_info a routed call's kwargs carry; a step without a deployment id leaves model_info empty
fn success_params(group: &str, step: &Step) -> JsonMap {
    let mut params = JsonMap::new();
    if step.metadata.unwrap_or(true) {
        let metadata =
            JsonObject::from_iter([("model_group".to_owned(), Json::Text(group.to_owned()))]);
        params.insert("metadata".to_owned(), Json::Object(metadata));
    }
    let mut info = JsonMap::new();
    if let Some(id) = &step.deployment_id {
        info.insert("id".to_owned(), Json::Text(id.clone()));
    }
    params.insert("model_info".to_owned(), Json::Object(info));
    params
}

fn selected(
    windows: &Windows,
    input: &Input,
    deployments: &[DeploymentSpec],
    step: &Step,
    now: DateTime<Utc>,
) -> Result<Option<String>, ScenarioError> {
    let pool = healthy_pool(deployments, &step.healthy)?;
    // the estimate the request computed is a failure's zero when it could not be counted
    let input_tokens = if step.token_count_failure.unwrap_or(false) {
        0
    } else {
        step.input_tokens.unwrap_or(0)
    };
    let picked = select(windows, &input.group, &pool, input_tokens, now).map_err(invalid)?;
    Ok(picked
        .and_then(|index| pool.get(index))
        .and_then(DeploymentSpec::id)
        .map(str::to_owned))
}

fn healthy_pool(
    deployments: &[DeploymentSpec],
    healthy: &[String],
) -> Result<Vec<DeploymentSpec>, ScenarioError> {
    if healthy.is_empty() {
        return Ok(deployments.to_vec());
    }
    healthy
        .iter()
        .map(|id| {
            deployments
                .iter()
                .find(|deployment| deployment.id() == Some(id.as_str()))
                .cloned()
                .ok_or_else(|| invalid(format!("{id} is not a deployment of the case")))
        })
        .collect()
}

fn observed(windows: &Windows, input: &Input, now: DateTime<Utc>) -> Observation {
    let (tokens_key, requests_key) = window_keys(&input.group, now);
    let (tpm, tpm_ttl) = window(&tokens_key, windows);
    let (rpm, rpm_ttl) = window(&requests_key, windows);
    Observation::Recorded {
        tpm,
        rpm,
        tpm_ttl,
        rpm_ttl,
    }
}

// a read the cache fails reports the key as absent, which is what DualCache hands back after it swallows one
fn window(key: &str, windows: &Windows) -> (Option<Vec<(String, i64)>>, Option<i64>) {
    match windows.read(key) {
        Ok(Some(counts)) => (
            Some(counts.into_iter().collect()),
            windows
                .ttls
                .lock()
                .ok()
                .and_then(|ttls| ttls.get(key).copied()),
        ),
        Ok(None) | Err(_) => (None, None),
    }
}

fn timestamp(millis: i64) -> Result<DateTime<Utc>, ScenarioError> {
    DateTime::from_timestamp_millis(millis).ok_or_else(|| {
        invalid(format!(
            "{millis} milliseconds is not an instant chrono can hold"
        ))
    })
}

fn invalid<E>(error: E) -> ScenarioError
where
    E: Display,
{
    ScenarioError::Invalid {
        message: error.to_string(),
    }
}
