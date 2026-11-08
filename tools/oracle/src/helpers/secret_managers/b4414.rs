mod script;

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_context::aws_partition::get_aws_partition;
use betterlite_secret_managers::aws::request_signing::AwsOptionalParams;
use betterlite_secret_managers::aws::{AwsDependencies, AwsSecretsManager};
use betterlite_secret_managers::aws_secret_reads::AwsReadError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};
use script::{CredentialFailure, Refused, Requests, Resolver, Route, Script};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    env: BTreeMap<String, String>,
    routes: Vec<Route>,
    steps: Vec<Step>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Path {
    Async,
    Sync,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    aws_region_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Read {
        path: Path,
        secret: String,
        primary: Option<String>,
        params: Option<Params>,
    },
    ReadPrimary {
        path: Path,
        secret: String,
        primary: String,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| {
                Box::new(HelperError {
                    class: "RuntimeError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        Ok(runtime.block_on(execute(input)))
    }
}

async fn execute(input: &Input) -> Value {
    let requests = Requests::default();
    let script = Script::new(&input.routes, Arc::clone(&requests));
    let manager = AwsSecretsManager {
        aws_region_name: None,
        aws_role_name: None,
        aws_session_name: None,
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        replica_regions: Vec::new(),
        kms_key_id: None,
        dependencies: AwsDependencies {
            environment: input.env.clone(),
            transport: Arc::new(script),
            resolver: Resolver {
                environment: input.env.clone(),
            },
            dns_suffix: |region: &str| get_aws_partition(Some(region)).dns_suffix,
            clock: signing_time,
        },
    };
    let mut steps = Vec::new();
    for step in &input.steps {
        let result = run_step(&manager, step).await;
        let sent = std::mem::take(&mut *requests.lock().unwrap_or_else(PoisonError::into_inner));
        steps.push(json!({"result": result, "requests": sent}));
    }
    json!({"steps": steps})
}

async fn run_step(manager: &AwsSecretsManager<Resolver, Script>, step: &Step) -> Value {
    match step {
        Step::Read {
            path,
            secret,
            primary,
            params,
        } => run_read(manager, *path, secret, primary.as_deref(), params.as_ref()).await,
        Step::ReadPrimary {
            path,
            secret,
            primary,
        } => match path {
            Path::Async => read_value(manager.async_read_from_primary(secret, primary).await),
            Path::Sync => read_value(manager.sync_read_from_primary(secret, primary)),
        },
    }
}

async fn run_read(
    manager: &AwsSecretsManager<Resolver, Script>,
    path: Path,
    secret: &str,
    primary: Option<&str>,
    params: Option<&Params>,
) -> Value {
    let mut optional = params.map(|settings| AwsOptionalParams {
        aws_role_name: None,
        aws_session_name: None,
        aws_region_name: settings.aws_region_name.clone(),
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_bedrock_runtime_endpoint: None,
    });
    match path {
        Path::Async => read_value(
            manager
                .async_read_secret(secret, optional.as_mut(), None, primary)
                .await,
        ),
        Path::Sync => {
            read_value(manager.sync_read_secret(secret, optional.as_mut(), None, primary))
        }
    }
}

fn read_value(
    result: Result<Option<SecretString>, AwsReadError<CredentialFailure, Refused>>,
) -> Value {
    match result {
        Ok(Some(secret)) => json!({"value": secret.expose_secret()}),
        Ok(None) => json!({"value": null}),
        Err(_) => json!({"raised": true}),
    }
}

fn signing_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_709_641_845))
        .unwrap_or(UNIX_EPOCH)
}
