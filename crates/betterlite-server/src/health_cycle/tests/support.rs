#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::routing_value_store::Clock;
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_json::Number;
use tokio::time::sleep;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::health_cycle::catalog::HealthCatalog;
use crate::health_cycle::cycle::{HealthCheckRequest, HealthCycleDeps, perform_health_check};
use crate::health_cycle::endpoints::HealthCheckResult;
use crate::health_cycle::error::HealthCycleError;
use crate::health_cycle::ports::{ProbeFailure, ProbeRequest, ProbeResponse, ProviderHealth};
use crate::health_cycle::probe::{PreparedProbe, prepare_probe};
use crate::health_cycle::telemetry::InstrumentationContext;
use crate::health_probe_settings::model_info::HealthCheckModelInfo;

pub(crate) fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(crate) fn deployment(name: &str, id: &str, params: &str) -> DeploymentSpec {
    deployment_with_info(name, id, params, "{}")
}

// A deployment whose litellm_params and model_info are written as proxy config JSON, with the id the run files it under
pub(crate) fn deployment_with_info(
    name: &str,
    id: &str,
    params: &str,
    info: &str,
) -> DeploymentSpec {
    let litellm_params: JsonObject = serde_json::from_str(params)
        .unwrap_or_else(|error| panic!("the fixture params are not valid JSON: {error}"));
    let mut model_info: JsonObject = serde_json::from_str(info)
        .unwrap_or_else(|error| panic!("the fixture model_info is not valid JSON: {error}"));
    model_info.insert("id".to_owned(), text(id));
    DeploymentSpec::new(
        name.to_owned(),
        litellm_params,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap_or_else(|error| panic!("the fixture deployment is invalid: {error}"))
}

pub(crate) fn with_timeout(mut deployment: DeploymentSpec, seconds: f64) -> DeploymentSpec {
    let value = Number::from_f64(seconds).expect("a finite fixture timeout");
    deployment
        .model_info
        .insert("health_check_timeout".to_owned(), Json::Number(value));
    deployment
}

#[derive(Default)]
pub(crate) struct FakeModels {
    pub(crate) modes: BTreeMap<String, &'static str>,
    pub(crate) reasoning: BTreeSet<String>,
}

impl HealthCheckModelInfo for FakeModels {
    fn mode(&self, model: &str) -> Option<String> {
        self.modes.get(model).map(|mode| (*mode).to_owned())
    }

    fn supports_reasoning(&self, model: &str) -> bool {
        self.reasoning.contains(model)
    }
}

pub(crate) fn policy() -> ProbePolicy {
    ProbePolicy::default()
}

pub(crate) fn policy_with_timeout(seconds: u64) -> ProbePolicy {
    ProbePolicy {
        skip_disabled_models: false,
        model_groups: None,
        max_tokens: None,
        max_tokens_reasoning: None,
        timeout: Duration::from_secs(seconds),
        prompt: "test from litellm".to_owned(),
    }
}

pub(crate) fn prepare_all(rows: &[DeploymentSpec]) -> Vec<PreparedProbe> {
    let models = FakeModels::default();
    let policy = policy();
    let mut random = seeded_random();
    rows.iter()
        .map(|row| prepare_probe(row, &models, &policy, &mut random))
        .collect()
}

// perform_health_check with the usual injected dependencies; the caller keeps the request's own borrows alive
pub(crate) async fn cycle(
    request: HealthCheckRequest<'_>,
    provider: &FakeProvider,
    models: &FakeModels,
    policy: &ProbePolicy,
) -> Result<HealthCheckResult, HealthCycleError> {
    let mut random = seeded_random();
    let deps = HealthCycleDeps {
        provider,
        models,
        policy,
        random: &mut random,
    };
    perform_health_check(request, deps).await
}

pub(crate) fn seeded_random() -> StdRng {
    StdRng::seed_from_u64(7)
}

pub(crate) fn fixed_clock(at: f64) -> Clock {
    Arc::new(move || at)
}

pub(crate) fn counting_clock(at: f64) -> (Clock, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let clock: Clock = Arc::new(move || {
        counter.fetch_add(1, Ordering::Relaxed);
        at
    });
    (clock, calls)
}

pub(crate) fn context(cycle_id: &str, source: &str) -> InstrumentationContext {
    InstrumentationContext {
        enabled: true,
        cycle_id: cycle_id.to_owned(),
        source: source.to_owned(),
    }
}

pub(crate) fn plain_request(model_list: &[DeploymentSpec]) -> HealthCheckRequest<'_> {
    HealthCheckRequest {
        model_list,
        model: None,
        model_id: None,
        team_id: None,
        cli_model: None,
        details: true,
        max_concurrency: None,
        skip_disabled: false,
        lookup: None,
        instrumentation: None,
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Reply {
    Success,
    Failure {
        status: u16,
        message: &'static str,
    },
    DelayedSuccess(Duration),
    DelayedFailure {
        delay: Duration,
        status: u16,
        message: &'static str,
    },
}

#[derive(Default)]
struct ProviderState {
    replies: BTreeMap<String, Reply>,
    calls: Vec<String>,
    requests: Vec<ProbeRequest>,
    running: usize,
    peak: usize,
}

pub(crate) struct FakeProvider {
    state: Mutex<ProviderState>,
}

impl FakeProvider {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(ProviderState::default()),
        }
    }

    pub(crate) fn with_reply(self, model: &str, reply: Reply) -> Self {
        self.state
            .lock()
            .unwrap()
            .replies
            .insert(model.to_owned(), reply);
        self
    }

    // The litellm model of every probe, in the order the provider started them
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    pub(crate) fn peak(&self) -> usize {
        self.state.lock().unwrap().peak
    }

    pub(crate) fn requests(&self) -> Vec<ProbeRequest> {
        self.state.lock().unwrap().requests.clone()
    }

    fn enter(&self, model: &str) -> Reply {
        let mut state = self.state.lock().unwrap();
        state.calls.push(model.to_owned());
        state.running = state.running.saturating_add(1);
        state.peak = state.peak.max(state.running);
        state.replies.get(model).cloned().unwrap_or(Reply::Success)
    }

    fn leave(&self) {
        let mut state = self.state.lock().unwrap();
        state.running = state.running.saturating_sub(1);
    }
}

pub(crate) fn text_field(endpoint: &JsonObject, key: &str) -> Option<String> {
    match endpoint.get(key) {
        Some(Json::Text(value)) => Some(value.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

pub(crate) fn model_of(params: &JsonObject) -> String {
    match params.get("model") {
        Some(Json::Text(model)) => model.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    }
}

pub(crate) fn success(model: &str) -> ProbeResponse {
    ProbeResponse {
        remaining_requests: Some(model.to_owned()),
        remaining_tokens: None,
        region: None,
    }
}

pub(crate) fn failure(status: u16, message: &'static str) -> ProbeFailure {
    ProbeFailure {
        message: message.to_owned(),
        status_code: status,
        raw_request_typed_dict: None,
    }
}

#[async_trait]
impl ProviderHealth for FakeProvider {
    async fn check(&self, request: ProbeRequest) -> Result<ProbeResponse, ProbeFailure> {
        let model = model_of(&request.params);
        self.state.lock().unwrap().requests.push(request.clone());
        let reply = self.enter(&model);
        let outcome = match reply {
            Reply::Success => Ok(success(&model)),
            Reply::Failure { status, message } => Err(failure(status, message)),
            Reply::DelayedSuccess(delay) => {
                sleep(delay).await;
                Ok(success(&model))
            }
            Reply::DelayedFailure {
                delay,
                status,
                message,
            } => {
                sleep(delay).await;
                Err(failure(status, message))
            }
        };
        self.leave();
        outcome
    }
}

type ProviderFn = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

// A catalog as the health run reads it: every deployment under its indices, the alias map beside them and no routing groups
pub(crate) struct Fixture {
    pub(crate) catalog: DeploymentCatalog,
    pub(crate) aliases: AliasMap,
    pub(crate) groups: RoutingGroupIndex,
    provider: ProviderFn,
}

impl Fixture {
    pub(crate) fn new(deployments: &[DeploymentSpec]) -> Self {
        Self::with_alias_map(deployments, AliasMap::new())
    }

    pub(crate) fn with_alias_map(deployments: &[DeploymentSpec], aliases: AliasMap) -> Self {
        let mut catalog = DeploymentCatalog::default();
        for row in deployments {
            catalog.model_names.insert(row.model_name.clone());
            catalog
                .index_patterns(row)
                .unwrap_or_else(|error| panic!("the fixture deployment is not indexable: {error}"));
            catalog.push(row.clone());
        }
        Self {
            catalog,
            aliases,
            groups: RoutingGroupIndex::default(),
            provider: Box::new(|_: &str| None),
        }
    }

    pub(crate) fn lookup(&self) -> HealthCatalog<'_> {
        HealthCatalog {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &*self.provider,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).expect("utf-8 log output")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
