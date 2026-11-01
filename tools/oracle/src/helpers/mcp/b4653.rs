use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_mcp::bridge_credentials::{self, ENCRYPTION_KEY_DOMAIN, SIGNING_KEY_DOMAIN};
use betterlite_mcp::session_credentials::{self, SESSION_SIGNING_KEY_DOMAIN};

use crate::helpers::{Helper, HelperError, HelperResult};

use bridge::EnvelopeFields;
use session::SessionFields;
use signing_config::SigningConfigFields;

mod bridge;
mod session;
mod signing_config;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Scrypt {
                master_key,
                domains,
            } => Ok(run_scrypt(master_key, domains)?),
            Input::BearerPrefixStrip { checks } => Ok(run_prefix_strip(checks)),
            Input::BridgeEnvelope(fields) => {
                bridge::run_bridge(fields, bridge::BridgeOperation::Envelope)
            }
            Input::BridgeRefresh(fields) => {
                bridge::run_bridge(fields, bridge::BridgeOperation::Refresh)
            }
            Input::SessionBearer(fields) => {
                session::run_session(fields, session::SessionOperation::Bearer)
            }
            Input::SessionRefresh(fields) => {
                session::run_session(fields, session::SessionOperation::Refresh)
            }
            Input::SigningConfig(fields) => Ok(signing_config::run_signing_config(fields)),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Scrypt {
        master_key: String,
        domains: Vec<ScryptDomain>,
    },
    BearerPrefixStrip {
        checks: Vec<PrefixCheck>,
    },
    BridgeEnvelope(EnvelopeFields),
    BridgeRefresh(EnvelopeFields),
    SessionBearer(SessionFields),
    SessionRefresh(SessionFields),
    SigningConfig(SigningConfigFields),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScryptDomain {
    EnvelopeSigning,
    EnvelopeEncryption,
    SessionSigning,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrefixCheck {
    family: PrefixFamily,
    candidate: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PrefixFamily {
    Bridge,
    Session,
}

pub(crate) fn helper_error<M>(class: &str, value: &M) -> Box<HelperError>
where
    M: ToString,
{
    let message = value.to_string();
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn run_scrypt(master_key: &str, domains: &[ScryptDomain]) -> Result<Value, Box<HelperError>> {
    let hex = domains
        .iter()
        .map(|domain| {
            let salt = match domain {
                ScryptDomain::EnvelopeSigning => SIGNING_KEY_DOMAIN,
                ScryptDomain::EnvelopeEncryption => ENCRYPTION_KEY_DOMAIN,
                ScryptDomain::SessionSigning => SESSION_SIGNING_KEY_DOMAIN,
            };
            bridge_credentials::scrypt_hex(master_key, salt)
                .map_err(|error| helper_error("ScryptError", &error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "hex": hex }))
}

fn run_prefix_strip(checks: &[PrefixCheck]) -> Value {
    let results = checks
        .iter()
        .map(|check| {
            let shaped = |candidate: &str| match check.family {
                PrefixFamily::Bridge => bridge_credentials::is_bridge_envelope_shaped(candidate),
                PrefixFamily::Session => session_credentials::is_session_bearer_shaped(candidate),
            };
            json!({
                "bare": shaped(&check.candidate),
                "bearer": shaped(&format!("Bearer {}", check.candidate)),
                "mixed_case": shaped(&format!("BEARER {}", check.candidate)),
            })
        })
        .collect::<Vec<_>>();
    json!({ "results": results })
}
