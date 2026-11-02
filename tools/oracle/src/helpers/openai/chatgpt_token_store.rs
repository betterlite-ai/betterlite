use std::time::Duration;

use betterlite_openai::chatgpt::token_store::member::Member;
use betterlite_openai::chatgpt::token_store::{ChatGptTokenStore, codec};
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

mod effects;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Read,
    Expired,
    Account,
    Build,
    Cooldown,
    Record,
    Wait(f64),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    record: Option<String>,
    replacement: Option<String>,
    now: f64,
    #[serde(default)]
    fail_write: bool,
    actions: Vec<Action>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        run_fixture(input).map_err(|error| {
            Box::new(HelperError {
                class: "TokenStoreFixture".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn run_fixture(input: &Input) -> Result<serde_json::Value, effects::FixtureError> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("auth.json");
    if let Some(record) = &input.record {
        std::fs::write(&path, record)?;
    }
    let mut store = effects::store(input, directory.path())?;
    let mut results = Vec::new();
    for action in &input.actions {
        results.push(run_action(&mut store, action)?);
    }
    if let Some(error) = store.clock.failure {
        return Err(effects::FixtureError::Io(error));
    }
    let content = std::fs::read_to_string(path).ok();
    Ok(
        json!({ "results": results, "file": effects::content(content.as_deref()), "writes": store.files.writes, "reads": store.files.reads, "events": store.files.events, "sleeps": store.clock.sleeps }),
    )
}

fn run_action(
    store: &mut ChatGptTokenStore<effects::Files, effects::ScriptClock>,
    action: &Action,
) -> Result<serde_json::Value, effects::FixtureError> {
    Ok(match action {
        Action::Read => json!(store.read()),
        Action::Account => json!(store.get_account_id()),
        Action::Cooldown => {
            let record = store.read();
            json!(store.cooldown_remaining(record.as_ref()).to_f64())
        }
        Action::Record => {
            store.record_device_request();
            json!(null)
        }
        Action::Build => {
            let record = store.read().unwrap_or_default();
            json!(codec::build_auth_record(record))
        }
        Action::Expired => {
            let mut record = store.read().unwrap_or_default();
            let expired = record
                .access_token
                .as_ref()
                .and_then(Member::known)
                .cloned()
                .is_none_or(|token| store.is_expired(&mut record, &token));
            json!(expired)
        }
        Action::Wait(seconds) => {
            let timeout = Duration::try_from_secs_f64(*seconds)?;
            json!(
                store
                    .wait_for_access_token(timeout)
                    .as_ref()
                    .map(secrecy::SecretString::expose_secret)
            )
        }
    })
}
