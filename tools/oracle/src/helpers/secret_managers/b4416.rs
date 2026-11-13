mod script;

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_secret_managers::vault::HashicorpSecretManager;
use betterlite_secret_managers::vault::error::{VaultError, VaultInitError};
use betterlite_secret_managers::vault::response::secret_value;
use betterlite_secret_managers::vault::target::VaultSecretParams;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};
use script::{Refused, Requests, Route, Script};

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
    Token,
    TokenBlocking,
    Namespaces,
    Target {
        name: String,
        #[serde(default)]
        params: Option<VaultSecretParams>,
    },
    Value {
        body: String,
        data_key: String,
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
        Ok(runtime.block_on(execute(input)))
    }
}

async fn execute(input: &Input) -> Value {
    let requests = Requests::default();
    let script = Script::new(&input.routes, Arc::clone(&requests));
    let clock = Arc::new(tokio::time::Instant::now);
    let manager = match HashicorpSecretManager::new(&input.env, script, clock) {
        Ok(manager) => manager,
        Err(error) => {
            return json!({"init": {"ok": false, "error": init_tag(&error)}, "steps": []});
        }
    };
    let mut steps = Vec::new();
    for step in &input.steps {
        let result = run_step(&manager, step).await;
        let sent = std::mem::take(&mut *requests.lock().unwrap_or_else(PoisonError::into_inner));
        steps.push(json!({"result": result, "requests": sent}));
    }
    json!({"init": {"ok": true}, "steps": steps})
}

const fn init_tag(error: &VaultInitError) -> &'static str {
    match error {
        VaultInitError::MissingCredentials => "missing_credentials",
        VaultInitError::RefreshInterval { .. } => "invalid_refresh_interval",
    }
}

async fn run_step(manager: &HashicorpSecretManager<Script>, step: &Step) -> Value {
    match step {
        Step::Token => login(manager.request_token().await),
        Step::TokenBlocking => login(manager.request_token_blocking()),
        Step::Namespaces => json!({
            "login": manager.vault_login_namespace(),
            "secret": manager.vault_secret_namespace(),
        }),
        Step::Target { name, params } => match manager.build_secret_target(name, params.as_ref()) {
            Ok(target) => json!({
                "url": target.url,
                "data_key": target.data_key,
                "secret_name": target.secret_name,
            }),
            Err(_error) => json!({"error": "unsafe_secret_name"}),
        },
        Step::Value { body, data_key } => json!({
            "value": secret_value(body.as_bytes(), data_key)
                .map(|value| value.expose_secret().to_owned()),
        }),
        Step::Advance { ms } => {
            tokio::time::advance(Duration::from_millis(*ms)).await;
            Value::Null
        }
    }
}

fn login(reply: Result<SecretString, VaultError<Refused>>) -> Value {
    match reply {
        Ok(token) => json!({"token": token.expose_secret()}),
        Err(_error) => json!({"error": "login_failed"}),
    }
}
