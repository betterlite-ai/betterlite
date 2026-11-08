mod outcome;
mod script;

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_secret_managers::secret_contracts::BaseSecretManager as _;
use betterlite_secret_managers::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretTimeout, SecretWrite,
};
use betterlite_secret_managers::vault::HashicorpSecretManager;
use betterlite_secret_managers::vault::target::VaultSecretParams;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};
use outcome::{deletion_outcome, infallible, operation_outcome, read_outcome, rotation_outcome};
use script::{Requests, Route, Script};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    env: BTreeMap<String, String>,
    routes: Vec<Route>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Read {
        name: String,
        #[serde(default)]
        params: Option<VaultSecretParams>,
        #[serde(default)]
        timeout: Option<f64>,
    },
    SyncRead {
        name: String,
        #[serde(default)]
        params: Option<VaultSecretParams>,
        #[serde(default)]
        timeout: Option<f64>,
    },
    Write {
        name: String,
        value: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        params: Option<VaultSecretParams>,
        #[serde(default)]
        timeout: Option<f64>,
    },
    Delete {
        name: String,
        #[serde(default)]
        params: Option<VaultSecretParams>,
        #[serde(default)]
        timeout: Option<f64>,
    },
    Rotate {
        current: String,
        new: String,
        value: String,
        #[serde(default)]
        params: Option<VaultSecretParams>,
        #[serde(default)]
        timeout: Option<f64>,
    },
}

impl Step {
    fn options(&self) -> Result<SecretOptions<'_, VaultSecretParams>, Box<HelperError>> {
        let (params, timeout) = match self {
            Self::Read {
                params, timeout, ..
            }
            | Self::SyncRead {
                params, timeout, ..
            }
            | Self::Write {
                params, timeout, ..
            }
            | Self::Delete {
                params, timeout, ..
            }
            | Self::Rotate {
                params, timeout, ..
            } => (params.as_ref(), *timeout),
        };
        Ok(SecretOptions {
            optional_params: params,
            timeout: timeout_limit(timeout)?,
        })
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(|source| {
                Box::new(HelperError {
                    message: source.to_string(),
                    class: "RuntimeError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                })
            })?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> HelperResult {
    let requests = Requests::default();
    let script = Script::new(&input.routes, Arc::clone(&requests));
    let clock = Arc::new(tokio::time::Instant::now);
    let manager = match HashicorpSecretManager::new(&input.env, script, clock) {
        Ok(manager) => manager,
        Err(_error) => return Ok(json!({"init": {"ok": false}, "steps": []})),
    };
    let mut steps = Vec::new();
    for step in &input.steps {
        let result = run_step(&manager, step).await?;
        let sent = std::mem::take(&mut *requests.lock().unwrap_or_else(PoisonError::into_inner));
        steps.push(json!({"result": result, "requests": sent}));
    }
    Ok(json!({"init": {"ok": true}, "steps": steps}))
}

async fn run_step(manager: &HashicorpSecretManager<Script>, step: &Step) -> HelperResult {
    let options = step.options()?;
    match step {
        Step::Read { name, .. } => Ok(read_outcome(infallible(
            manager.async_read_secret(name, options).await,
        ))),
        Step::SyncRead { name, .. } => Ok(read_outcome(infallible(
            manager.sync_read_secret(name, options),
        ))),
        Step::Write {
            name,
            value,
            description,
            ..
        } => write_step(manager, name, value, description.as_deref(), options).await,
        Step::Delete { name, .. } => delete_step(manager, name, options).await,
        Step::Rotate {
            current,
            new,
            value,
            ..
        } => rotate_step(manager, current, new, value, options).await,
    }
}

async fn write_step(
    manager: &HashicorpSecretManager<Script>,
    name: &str,
    value: &str,
    description: Option<&str>,
    options: SecretOptions<'_, VaultSecretParams>,
) -> HelperResult {
    let secret_value = SecretString::from(value);
    let written = manager
        .async_write_secret(SecretWrite {
            secret_name: name,
            secret_value: &secret_value,
            description,
            options,
            tags: None,
        })
        .await;
    Ok(operation_outcome(&infallible(written)))
}

async fn delete_step(
    manager: &HashicorpSecretManager<Script>,
    name: &str,
    options: SecretOptions<'_, VaultSecretParams>,
) -> HelperResult {
    let deleted = manager
        .async_delete_secret(
            name,
            DeleteOptions {
                recovery_window_in_days: 7,
                operation: options,
            },
        )
        .await;
    Ok(deletion_outcome(&infallible(deleted)))
}

async fn rotate_step(
    manager: &HashicorpSecretManager<Script>,
    current: &str,
    new: &str,
    value: &str,
    options: SecretOptions<'_, VaultSecretParams>,
) -> HelperResult {
    let secret_value = SecretString::from(value);
    let rotated = manager
        .async_rotate_secret(SecretRotation {
            current_secret_name: current,
            new_secret_name: new,
            new_secret_value: &secret_value,
            options,
        })
        .await;
    Ok(rotation_outcome(rotated))
}

fn timeout_limit(seconds: Option<f64>) -> Result<Option<SecretTimeout>, Box<HelperError>> {
    let Some(limit) = seconds else {
        return Ok(None);
    };
    let duration = Duration::try_from_secs_f64(limit).map_err(|source| {
        Box::new(HelperError {
            message: source.to_string(),
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
        })
    })?;
    Ok(Some(SecretTimeout::Total(duration)))
}
