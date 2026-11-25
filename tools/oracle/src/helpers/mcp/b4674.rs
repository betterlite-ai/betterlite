mod claims;
mod open;
mod register;
mod tests;

use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_db::mcp_stored_contracts::JsonValue as DbJsonValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

const SALT: &str = "b4674-oracle-salt";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    register: Option<Vec<RegisterCall>>,
    open: Option<Vec<OpenScenario>>,
    claims: Option<Vec<ClaimScenario>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegisterCall {
    now: i64,
    body: BTreeMap<String, DbJsonValue>,
    token_exchange_available: bool,
    native_redirect_uris: Option<String>,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum SealPrefix {
    Client,
    Code,
    #[serde(rename = "none")]
    Bare,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Wrap {
    None,
    Tamper,
    Truncate,
    SwapPrefix,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenScenario {
    name: String,
    payload: Option<Value>,
    prefix: SealPrefix,
    wrap: Wrap,
    raw: Option<String>,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Authority {
    Local,
    Shared,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum BackendKind {
    Counting,
    FixedZero,
    FixedOne,
    FixedTwo,
    Faulty,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Action {
    Claim,
    Peek,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimStep {
    action: Action,
    key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimScenario {
    name: String,
    authority: Authority,
    backend: BackendKind,
    ttl: i64,
    #[serde(default)]
    seed_keys: Vec<String>,
    steps: Vec<ClaimStep>,
}
pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cipher = cipher();
        let registers = input
            .register
            .as_deref()
            .map(|calls| register::reports(calls, &cipher))
            .transpose()?;
        let opens = input
            .open
            .as_deref()
            .map(|scenarios| open::reports(scenarios, &cipher))
            .transpose()?;
        let claims = match &input.claims {
            Some(scenarios) => {
                let runtime = Builder::new_current_thread().build().map_err(failure)?;
                Some(runtime.block_on(claims::reports(scenarios)))
            }
            None => None,
        };
        Ok(json!({"register": registers, "open": opens, "claims": claims}))
    }
}

fn failure<E>(error: E) -> Box<HelperError>
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

fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(SALT)),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}
