use std::collections::BTreeMap;
use std::future::ready;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_bedrock::vector_stores::signing::{SignRequest, sign_request};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;
use serde_json::json;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use super::SignScenario;
use super::operations::message_failure;
use crate::helpers::HelperError;

pub(super) fn sign_outcome(scenario: &SignScenario) -> Result<Value, Box<HelperError>> {
    let params = raw_params(&scenario.optional_params)?;
    let api_key = scenario
        .api_key
        .as_ref()
        .map(|key| SecretString::from(key.clone()));
    let configured = scenario
        .configured_bearer
        .as_ref()
        .map(|token| SecretString::from(token.clone()));
    let now = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(scenario.now))
        .ok_or_else(|| message_failure("the signing time is out of range"))?;
    let credentials = Credentials {
        access_key_id: SecretString::from(scenario.credentials.access_key_id.clone()),
        secret_access_key: SecretString::from(scenario.credentials.secret_access_key.clone()),
        session_token: scenario
            .credentials
            .session_token
            .clone()
            .map(SecretString::from),
    };
    let signed = runtime()?.block_on(sign_request(
        SignRequest {
            headers: &scenario.headers,
            optional_params: &params,
            request_data: &scenario.request_data,
            api_base: &scenario.api_base,
            api_key: api_key.as_ref(),
            configured_bearer: configured.as_ref(),
            region_inputs: RegionInputs::default(),
            now,
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(credentials)),
    ));
    signed.map_or_else(|_error| Ok(json!({"failed": true})), signed_value)
}

fn signed_value(signed: AwsSignedRequest) -> Result<Value, Box<HelperError>> {
    let AwsSignedRequest {
        url, headers, body, ..
    } = signed;
    let text = String::from_utf8(body)
        .map_err(|error| message_failure(&format!("the signed body is not UTF-8: {error}")))?;
    let mut pairs: Vec<(String, String)> = headers
        .into_iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .collect();
    pairs.sort();
    Ok(json!({"url": url, "headers": pairs, "body": text}))
}

fn raw_params(
    params: &IndexMap<String, Json>,
) -> Result<BTreeMap<String, Box<RawValue>>, Box<HelperError>> {
    params
        .iter()
        .map(|(name, value)| {
            let text = serde_json::to_string(value).map_err(|error| {
                message_failure(&format!("the parameter {name} is not JSON: {error}"))
            })?;
            let raw = RawValue::from_string(text).map_err(|error| {
                message_failure(&format!("the parameter {name} is not raw JSON: {error}"))
            })?;
            Ok((name.clone(), raw))
        })
        .collect()
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| message_failure(&format!("the helper runtime cannot start: {error}")))
}
