mod logs;
mod script;

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_secret_managers::cyberark::CyberArkSecretManager;
use betterlite_secret_managers::cyberark::operation::{OperationStatus, SecretOperation};
use betterlite_secret_managers::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretWrite,
};
use betterlite_secret_managers::secret_contracts::{BaseSecretManager as _, RotationError};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};
use logs::Logs;
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
    },
    SyncRead {
        name: String,
    },
    Write {
        name: String,
        value: String,
    },
    Delete {
        name: String,
    },
    Rotate {
        current: String,
        new: String,
        value: String,
    },
    Advance {
        ms: u64,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
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
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let script = Script::new(&input.routes, Arc::clone(&requests));
    let Ok(manager) =
        CyberArkSecretManager::new(&input.env, script, Arc::new(tokio::time::Instant::now))
    else {
        return json!({"init": {"ok": false, "logs": logs.take()}, "steps": []});
    };
    let init = json!({"ok": true, "logs": logs.take()});
    let mut steps = Vec::new();
    for step in &input.steps {
        let result = run_step(&manager, step).await;
        let entries = logs.take();
        let sent = std::mem::take(&mut *requests.lock().unwrap_or_else(PoisonError::into_inner));
        steps.push(json!({"result": result, "logs": entries, "requests": sent}));
    }
    json!({"init": init, "steps": steps})
}

async fn run_step(manager: &CyberArkSecretManager<Script>, step: &Step) -> Value {
    match step {
        Step::Read { name } => {
            let Ok(value) = manager
                .async_read_secret(name, SecretOptions::default())
                .await;
            json!({"value": value.map(|secret| secret.expose_secret().to_owned())})
        }
        Step::SyncRead { name } => {
            let Ok(value) = manager.sync_read_secret(name, SecretOptions::default());
            json!({"value": value.map(|secret| secret.expose_secret().to_owned())})
        }
        Step::Write { name, value } => write(manager, name, value).await,
        Step::Delete { name } => {
            let Ok(deleted) = manager
                .async_delete_secret(name, DeleteOptions::default())
                .await;
            operation(&deleted)
        }
        Step::Rotate {
            current,
            new,
            value,
        } => rotate(manager, current, new, value).await,
        Step::Advance { ms } => {
            tokio::time::advance(Duration::from_millis(*ms)).await;
            Value::Null
        }
    }
}

async fn write(manager: &CyberArkSecretManager<Script>, name: &str, value: &str) -> Value {
    let secret_value = SecretString::from(value);
    let Ok(written) = manager
        .async_write_secret(SecretWrite {
            secret_name: name,
            secret_value: &secret_value,
            description: None,
            options: SecretOptions::default(),
            tags: None,
        })
        .await;
    operation(&written)
}

async fn rotate(
    manager: &CyberArkSecretManager<Script>,
    current: &str,
    new: &str,
    value: &str,
) -> Value {
    let secret_value = SecretString::from(value);
    let rotation = SecretRotation {
        current_secret_name: current,
        new_secret_name: new,
        new_secret_value: &secret_value,
        options: SecretOptions::default(),
    };
    match manager.async_rotate_secret(rotation).await {
        Ok(rotated) => operation(&rotated),
        Err(RotationError::CurrentSecretNotFound) => json!({"error": "current_secret_not_found"}),
        Err(RotationError::NewSecretNotVerified) => json!({"error": "new_secret_not_verified"}),
        Err(RotationError::SameName) => json!({"error": "same_name"}),
        Err(RotationError::Operation { source }) => match source {},
    }
}

fn operation(outcome: &SecretOperation) -> Value {
    match outcome.status {
        OperationStatus::Error => json!({"status": outcome.status, "message": null}),
        OperationStatus::Success | OperationStatus::NotSupported => {
            json!({"status": outcome.status, "message": outcome.message})
        }
    }
}
