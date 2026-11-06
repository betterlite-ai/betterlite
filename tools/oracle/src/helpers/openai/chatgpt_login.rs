use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, PoisonError};

use betterlite_openai::chatgpt::login::ChatGptLogin;
use betterlite_openai::chatgpt::login::error::LoginError;
use betterlite_openai::chatgpt::token_store::ChatGptTokenStore;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

mod effects;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    record: Option<String>,
    replacement: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    now: Decimal,
    replies: Vec<effects::Reply>,
    repeat: Option<effects::Reply>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        run_fixture(input).map_err(|error| {
            Box::new(HelperError {
                class: "LoginFixture".to_owned(),
                status: None,
                error_type: Some(None),
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("fixture IO failed")]
    Io(#[from] std::io::Error),
    #[error("fixture configuration failed")]
    Config(#[from] betterlite_config::loader::ConfigError),
    #[error("fixture store failed")]
    Store(#[from] betterlite_openai::chatgpt::token_store::StoreError),
}

const fn class(error: &LoginError) -> &'static str {
    match error {
        LoginError::DeviceCode(_) => "GetDeviceCodeError",
        LoginError::AccessToken(_) => "GetAccessTokenError",
        LoginError::Refresh(_) => "RefreshAccessTokenError",
    }
}

type Login = ChatGptLogin<effects::Files, effects::Time, effects::Script, effects::Shown>;

fn assemble(
    input: &Input,
    directory: &Path,
    events: &effects::Events,
) -> Result<Login, FixtureError> {
    let mut settings = betterlite_config::loader::ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")?
    .sections
    .chatgpt_token_store;
    settings.token_dir = directory.display().to_string();
    let files = effects::Files {
        writes: Vec::new(),
        events: Arc::clone(events),
    };
    let clock = effects::Time {
        now: input.now,
        sleeps: Vec::new(),
        path: directory.join("auth.json"),
        replacement: input.replacement.clone(),
        failure: None,
    };
    Ok(ChatGptLogin {
        store: ChatGptTokenStore::new(settings, files, clock)?,
        http: effects::Script {
            replies: input.replies.iter().cloned().collect(),
            repeat: input.repeat.clone(),
            requests: Vec::new(),
        },
        display: effects::Shown {
            prompts: Vec::new(),
        },
    })
}

fn summary(
    login: &Login,
    outcome: &Result<SecretString, LoginError>,
    path: &Path,
    events: &effects::Events,
) -> Value {
    let (token, error) = match outcome {
        Ok(token) => (Some(token.expose_secret()), None),
        Err(error) => (
            None,
            Some(json!({ "class": class(error), "status": error.status() })),
        ),
    };
    let file = std::fs::read_to_string(path)
        .ok()
        .map(|text| serde_json::from_str(&text).unwrap_or(Value::String(text)));
    let recorded = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    json!({
        "token": token,
        "error": error,
        "requests": login.http.requests,
        "sleeps": login.store.clock.sleeps,
        "file": file,
        "writes": login.store.files.writes,
        "events": recorded,
        "display": login.display.prompts,
    })
}

fn run_fixture(input: &Input) -> Result<Value, FixtureError> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("auth.json");
    if let Some(record) = &input.record {
        std::fs::write(&path, record)?;
    }
    let events: effects::Events = Arc::default();
    let mut login = assemble(input, directory.path(), &events)?;
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let subscriber = tracing_subscriber::registry().with(effects::Diagnostics(Arc::clone(&events)));
    let outcome = tracing::subscriber::with_default(subscriber, || {
        runtime.block_on(login.get_access_token())
    });
    if let Some(error) = login.store.clock.failure.take() {
        return Err(FixtureError::Io(error));
    }
    Ok(summary(&login, &outcome, &path, &events))
}
