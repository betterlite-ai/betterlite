use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::time::{Duration, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::responses::auth::{SignInputs, sign_request, validate_environment};
use betterlite_bedrock::responses::error::error_from_reply;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::input::{AuthCase, ErrorProbe, SigningCase};
use super::ops::{API_BASE, Case, runtime};

const CLOCK_SECONDS: u64 = 1_767_225_600;
const BEARER_VARIABLE: &str = "AWS_BEARER_TOKEN_BEDROCK";

fn static_credentials(
    settings: &AwsSettings,
    _region: &str,
) -> Ready<Result<Credentials, CredentialError>> {
    ready(
        match (&settings.access_key_id, &settings.secret_access_key) {
            (Some(access), Some(secret)) => Ok(Credentials {
                access_key_id: access.clone(),
                secret_access_key: secret.clone(),
                session_token: None,
            }),
            _ => Err(CredentialError::MissingCredentials),
        },
    )
}

fn secret_of(text: Option<&String>) -> Option<SecretString> {
    text.cloned().map(SecretString::from)
}

pub(super) fn auth_case(case: &AuthCase) -> Case {
    let api_key = secret_of(case.api_key.as_ref());
    let environment_token = secret_of(case.environment.get(BEARER_VARIABLE));
    let mut validated = case.headers.clone();
    validate_environment(&mut validated, api_key.as_ref(), environment_token.as_ref());
    Ok(json!({"headers": validated}))
}

fn error_value(probe: Option<&ErrorProbe>) -> Case {
    let Some(given) = probe else {
        return Ok(Value::Null);
    };
    let pairs: Vec<(String, String)> = given
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let error = error_from_reply(given.status, &given.message, &pairs).or_value_error()?;
    let request_id = error
        .response_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-amzn-requestid"))
        .map(|(_, value)| value.clone());
    Ok(json!({
        "status": error.error.status_code,
        "message": error.error.message,
        "request_id": request_id,
    }))
}

// The signing time and signature change on every run, so only their shape is compared
fn signed_summary(headers: &BTreeMap<String, Option<String>>) -> Case {
    let text = |name: &str| headers.get(name).cloned().flatten().unwrap_or_default();
    let authorization = text("Authorization");
    let (_, rest) = authorization
        .split_once("Credential=")
        .ok_or("no credential scope")
        .or_value_error()?;
    let (credential, rest) = rest
        .split_once(", SignedHeaders=")
        .ok_or("no signed headers")
        .or_value_error()?;
    let (signed_headers, _signature) = rest
        .split_once(", Signature=")
        .ok_or("no signature")
        .or_value_error()?;
    let fields: Vec<&str> = credential.split('/').collect();
    let [access_key, _date, region, service, _scope] = fields.as_slice() else {
        return Err("the credential scope has the wrong shape").or_value_error();
    };
    let plain: BTreeMap<&String, &Option<String>> = headers
        .iter()
        .filter(|(name, _)| !["Authorization", "X-Amz-Date"].contains(&name.as_str()))
        .collect();
    Ok(json!({
        "access_key": access_key,
        "region": region,
        "service": service,
        "signed_headers": signed_headers,
        "headers": plain,
    }))
}

pub(super) fn sigv4_case(case: &SigningCase) -> Case {
    let headers: Vec<(String, Option<String>)> = case
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect();
    let inputs = SignInputs {
        headers: &headers,
        optional_params: &case.optional_params,
        request_data: &case.request_data,
        api_base: API_BASE,
        model: None,
        api_key: None,
        environment_token: None,
        region_inputs: RegionInputs::default(),
        now: UNIX_EPOCH
            .checked_add(Duration::from_secs(CLOCK_SECONDS))
            .ok_or("the clock is out of range")
            .or_value_error()?,
    };
    let signed = runtime()?
        .block_on(sign_request(inputs, static_credentials))
        .or_value_error()?;
    let signed_headers: BTreeMap<String, Option<String>> = signed.headers.into_iter().collect();
    Ok(json!({
        "signed": signed_summary(&signed_headers)?,
        "body": signed.body.map(|bytes| STANDARD.encode(bytes)),
        "error": error_value(case.error.as_ref())?,
    }))
}
