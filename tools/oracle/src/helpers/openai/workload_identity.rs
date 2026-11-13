mod script;

use crate::helpers::OrHelperError as _;
use std::collections::BTreeMap;
use std::fs;
use std::mem;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_openai::workload_identity::auth::WorkloadIdentityAuth;
use betterlite_openai::workload_identity::cache::WorkloadIdentityCredentials;
use betterlite_openai::workload_identity::config::{
    OpenAiWorkloadIdentityConfig, WorkloadIdentityEnvironment, resolve_config,
};
use betterlite_openai::workload_identity::error::WorkloadIdentityError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::runtime::Builder;
use tokio::task;

use crate::helpers::{Helper, HelperError, HelperResult};
use script::{Reply, Script, Sdk, error_json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) enum Input {
    Resolve { probes: Vec<Probe> },
    Flow(Flow),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    env: BTreeMap<String, String>,
    api_key: Option<String>,
    api_base: Option<String>,
    litellm_api_base: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Flow {
    #[serde(default)]
    files: BTreeMap<String, String>,
    replies: Vec<Reply>,
    steps: Vec<Step>,
    #[serde(default)]
    sdk: Sdk,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    name: String,
    provider: Option<String>,
    account: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Token { identity: Identity },
    Concurrent { identity: Identity, callers: usize },
    Auth { identity: Identity, label: String },
    Advance { ms: u64 },
}

type Auth = Arc<WorkloadIdentityAuth<Script>>;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::Resolve { probes } => {
                let results: Result<Vec<Value>, _> = probes.iter().map(probe).collect();
                Ok(json!({ "results": results? }))
            }
            Input::Flow(flow) => run_flow(flow),
        }
    }
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn probe(fields: &Probe) -> Result<Value, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: fields.env.clone(),
    }
    .load("")
    .or_config_error()?;
    let api_key = fields.api_key.clone().map(SecretString::from);
    let found = resolve_config(
        api_key.as_ref(),
        fields.api_base.as_deref(),
        WorkloadIdentityEnvironment {
            env_api_key: config
                .sections
                .resource_sdk_settings
                .openai_api_key
                .as_ref(),
            litellm_api_base: fields.litellm_api_base.as_deref(),
            credentials: &config.sections.openai_credentials,
            identity: &config.sections.openai_workload_identity,
        },
    )
    .map_err(|error| invalid_input(error.to_string()))?;
    Ok(json!({ "config": found.map(|resolved| json!({
        "identity_provider_id": resolved.identity_provider_id,
        "service_account_id": resolved.service_account_id,
        "token_file": resolved.token_file,
    })) }))
}

fn run_flow(flow: &Flow) -> HelperResult {
    let directory = TempDir::new().map_err(|error| invalid_input(error.to_string()))?;
    for (name, contents) in &flow.files {
        fs::write(directory.path().join(name), contents)
            .map_err(|error| invalid_input(error.to_string()))?;
    }
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| invalid_input(error.to_string()))?;
    Ok(runtime.block_on(execute(flow, &directory)))
}

async fn execute(flow: &Flow, directory: &TempDir) -> Value {
    let script = Arc::new(Script::new(&flow.replies));
    let milliseconds = Arc::new(AtomicU64::new(0));
    let reader = Arc::clone(&milliseconds);
    let credentials = WorkloadIdentityCredentials::new(
        Arc::clone(&script),
        Arc::new(move || Duration::from_millis(reader.load(Ordering::Relaxed))),
        flow.sdk.capability(),
    );
    let mut labelled: Vec<(String, Auth)> = Vec::new();
    let mut steps = Vec::new();
    for step in &flow.steps {
        let result = run_step(&credentials, &milliseconds, directory, &mut labelled, step).await;
        steps.push(json!({
            "result": result,
            "requests": mem::take(&mut *script.requests.lock().unwrap_or_else(PoisonError::into_inner)),
        }));
    }
    json!({ "steps": steps })
}

async fn run_step(
    credentials: &WorkloadIdentityCredentials<Script>,
    milliseconds: &AtomicU64,
    directory: &TempDir,
    labelled: &mut Vec<(String, Auth)>,
    step: &Step,
) -> Value {
    let text = directory.path().to_string_lossy().into_owned();
    match step {
        Step::Advance { ms } => {
            milliseconds.fetch_add(*ms, Ordering::Relaxed);
            Value::Null
        }
        Step::Token { identity } => {
            let config = identity_config(&text, identity);
            outcome(token(credentials, &config).await, &text)
        }
        Step::Concurrent { identity, callers } => {
            let config = identity_config(&text, identity);
            concurrent(credentials, &config, *callers, &text).await
        }
        Step::Auth { identity, label } => {
            let config = identity_config(&text, identity);
            auth_result(credentials, &config, label, labelled, &text)
        }
    }
}

fn identity_config(directory: &str, identity: &Identity) -> OpenAiWorkloadIdentityConfig {
    OpenAiWorkloadIdentityConfig {
        identity_provider_id: identity
            .provider
            .clone()
            .unwrap_or_else(|| format!("idp-{}", identity.name)),
        service_account_id: identity
            .account
            .clone()
            .unwrap_or_else(|| format!("sa-{}", identity.name)),
        token_file: Path::new(directory)
            .join(&identity.name)
            .to_string_lossy()
            .into_owned(),
    }
}

fn outcome(result: Result<SecretString, WorkloadIdentityError>, directory: &str) -> Value {
    match result {
        Ok(token) => json!({ "token": token.expose_secret() }),
        Err(error) => json!({ "error": error_json(&error, directory) }),
    }
}

async fn token(
    credentials: &WorkloadIdentityCredentials<Script>,
    config: &OpenAiWorkloadIdentityConfig,
) -> Result<SecretString, WorkloadIdentityError> {
    credentials
        .workload_identity_auth(config)?
        .get_token()
        .await
}

async fn concurrent(
    credentials: &WorkloadIdentityCredentials<Script>,
    config: &OpenAiWorkloadIdentityConfig,
    callers: usize,
    text: &str,
) -> Value {
    let auth = match credentials.workload_identity_auth(config) {
        Ok(found) => found,
        Err(error) => {
            let failed = json!({ "error": error_json(&error, text) });
            return json!({ "outcomes": vec![failed; callers] });
        }
    };
    let mut running = Vec::new();
    for _ in 0..callers {
        let shared = Arc::clone(&auth);
        running.push(task::spawn(async move { shared.get_token().await }));
    }
    let mut outcomes = Vec::new();
    for handle in running {
        outcomes.push(match handle.await {
            Ok(result) => outcome(result, text),
            Err(error) => json!({ "error": { "kind": "internal", "message": error.to_string() } }),
        });
    }
    json!({ "outcomes": outcomes })
}

fn auth_result(
    credentials: &WorkloadIdentityCredentials<Script>,
    config: &OpenAiWorkloadIdentityConfig,
    label: &str,
    labelled: &mut Vec<(String, Auth)>,
    directory: &str,
) -> Value {
    let found = match credentials.workload_identity_auth(config) {
        Ok(auth) => auth,
        Err(error) => return json!({ "error": error_json(&error, directory) }),
    };
    let same_as: Vec<&str> = labelled
        .iter()
        .filter(|(_, earlier)| Arc::ptr_eq(earlier, &found))
        .map(|(name, _)| name.as_str())
        .collect();
    let result = json!({ "same_as": same_as });
    match labelled.iter_mut().find(|(name, _)| name == label) {
        Some(slot) => slot.1 = found,
        None => labelled.push((label.to_owned(), found)),
    }
    result
}

mod tests;
