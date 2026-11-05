use std::fmt::Display;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::password_policy::Settings;
use betterlite_server::password_policy::bulk::validate_passwords_bulk;
use betterlite_server::password_policy::error::{ERROR_TYPE, PARAM, PasswordError, STATUS};
use betterlite_server::password_policy::hashing::{HashError, PasswordHasher, SaltSource};
use betterlite_server::password_policy::strength;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};
use hibp::{Fixture, Hibp};
use logs::Logs;

mod hibp;
mod logs;
mod store;

const WORKERS: usize = 4;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct User {
    user_id: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Hash {
        password: String,
    },
    Verify {
        password: String,
        stored: String,
    },
    Strength {
        password: String,
        #[serde(default)]
        general_settings: Map<String, Value>,
    },
    Breach {
        password: String,
        #[serde(default)]
        general_settings: Map<String, Value>,
        hibp: Hibp,
    },
    Bulk {
        passwords: Vec<String>,
        #[serde(default)]
        general_settings: Map<String, Value>,
        hibp: Hibp,
    },
    Migrate {
        users: Vec<User>,
    },
}

// The same fixed salt on both sides makes the scrypt hashes comparable byte for byte.
struct FixedSalts;

impl SaltSource for FixedSalts {
    fn fill(&self, salt: &mut [u8; 16]) -> Result<(), HashError> {
        for (index, byte) in salt.iter_mut().enumerate() {
            *byte = u8::try_from(index).map_err(|_overflow| HashError::Params)?;
        }
        Ok(())
    }
}

fn failure<E>(error: &E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "HelperFailure".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn verdict(error: Option<&PasswordError>) -> Value {
    error.map_or(Value::Null, |failed| {
        json!({
            "status": STATUS,
            "type": ERROR_TYPE,
            "param": PARAM,
            "code": STATUS.to_string(),
            "message": failed.to_string(),
        })
    })
}

fn settings(general: &Map<String, Value>) -> Result<Settings, Box<HelperError>> {
    let source = json!({ "general_settings": general }).to_string();
    let loaded = ConfigLoader {
        lookup: std::collections::BTreeMap::new(),
    }
    .load(&source)
    .map_err(|error| failure(&error))?;
    Ok(loaded.sections.password_policy)
}

fn observed(fixture: &Fixture, logs: &Logs, extra: Value) -> Value {
    let (warnings, details) = logs.split();
    let mut value = json!({
        "requests": fixture.requests(),
        "warnings": warnings,
        "warning_details": details,
    });
    if let (Value::Object(fields), Value::Object(more)) = (&mut value, extra) {
        fields.extend(more);
    }
    value
}

async fn breach(
    password: &str,
    general: &Map<String, Value>,
    hibp: &Hibp,
    logs: &Logs,
) -> HelperResult {
    let resolved = settings(general)?;
    let fixture = hibp::start(hibp, resolved.check_breached_passwords).await?;
    let outcome = fixture
        .checker
        .validate_not_breached(&SecretString::from(password.to_owned()))
        .await;
    let extra = json!({
        "breached": outcome.is_err(),
        "error": verdict(outcome.err().as_ref()),
    });
    Ok(observed(&fixture, logs, extra))
}

async fn bulk(
    passwords: &[String],
    general: &Map<String, Value>,
    hibp: &Hibp,
    logs: &Logs,
) -> HelperResult {
    let resolved = settings(general)?;
    let fixture = hibp::start(hibp, resolved.check_breached_passwords).await?;
    let secrets: Vec<SecretString> = passwords.iter().cloned().map(SecretString::from).collect();
    let judged = validate_passwords_bulk(&secrets, &resolved, &fixture.checker).await;
    let verdicts: Vec<Value> = judged
        .iter()
        .map(|entry| {
            json!({
                "password": entry.password.expose_secret(),
                "error": verdict(entry.error.as_ref()),
            })
        })
        .collect();
    Ok(observed(&fixture, logs, json!({ "verdicts": verdicts })))
}

async fn run_function(input: &Input, logs: &Logs) -> HelperResult {
    let hasher = PasswordHasher::new(FixedSalts, WORKERS);
    match input {
        Input::Hash { password } => {
            let hash = hasher
                .hash(&SecretString::from(password.clone()))
                .await
                .map_err(|error| failure(&error))?;
            Ok(json!({ "hash": hash }))
        }
        Input::Verify { password, stored } => {
            let verified = hasher
                .verify(&SecretString::from(password.clone()), stored)
                .await;
            Ok(json!({ "verified": verified }))
        }
        Input::Strength {
            password,
            general_settings,
        } => {
            let outcome = strength::validate(
                &SecretString::from(password.clone()),
                &settings(general_settings)?,
            );
            Ok(json!({ "error": verdict(outcome.err().as_ref()) }))
        }
        Input::Breach {
            password,
            general_settings,
            hibp,
        } => breach(password, general_settings, hibp, logs).await,
        Input::Bulk {
            passwords,
            general_settings,
            hibp,
        } => bulk(passwords, general_settings, hibp, logs).await,
        Input::Migrate { users } => store::migrate(users, &hasher).await,
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(&error))?;
        let logs = Logs::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        with_default(subscriber, || runtime.block_on(run_function(input, &logs)))
    }
}
