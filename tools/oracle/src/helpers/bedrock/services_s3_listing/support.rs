use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::request_policy::{Params, RequestValue};
use betterlite_bedrock::s3_access::{S3AccessParams, TrustedValue, frozen_mapping};
use betterlite_bedrock::s3_listing::requests::S3FilesRequest;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use tokio::runtime::{Builder, Runtime};

use super::input::{Call, WireValue};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

// the Python side patches botocore's clock and pins the environment pair to these values
pub(super) const SIGNING_SECONDS: u64 = 1_704_067_200;
pub(super) const ENV_ACCESS_KEY: &str = "AKIDEXAMPLE";
pub(super) const ENV_KEY_VALUE: &str = "secret";

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread().build().or_value_error()
}

pub(super) fn signing_now() -> Result<SystemTime, Box<HelperError>> {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(SIGNING_SECONDS))
        .ok_or("the signing time is out of range")
        .or_value_error()
}

pub(super) fn to_params(map: &Map<String, Value>) -> Params {
    let mut params = Params::new();
    for (name, value) in map {
        let entry = match value {
            Value::String(text) => RequestValue::Text(text.clone()),
            Value::Object(object) => RequestValue::Mapping(to_params(object)),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => {
                RequestValue::Raw(value.to_string())
            }
        };
        params.insert(name.clone(), entry);
    }
    params
}

pub(super) fn from_params(params: &Params) -> HelperResult {
    let mut map = Map::new();
    for (name, value) in params {
        let entry = match value {
            RequestValue::Text(text) => Value::String(text.clone()),
            RequestValue::Secret(secret) => Value::String(secret.expose_secret().to_owned()),
            RequestValue::Mapping(mapping) => from_params(mapping)?,
            RequestValue::Raw(raw) => serde_json::from_str(raw).or_value_error()?,
        };
        map.insert(name.clone(), entry);
    }
    Ok(Value::Object(map))
}

fn trusted_value(value: &WireValue) -> TrustedValue {
    match value {
        WireValue::Text(text) => TrustedValue::Text(text.clone()),
        WireValue::Boolean(flag) => TrustedValue::Boolean(*flag),
        WireValue::Null(()) => TrustedValue::Null,
        WireValue::Other(_) => TrustedValue::Other,
    }
}

// the case snapshot is the trusted _litellm_internal_model_credentials; request buckets never authorize
pub(super) fn access_params(trusted: Option<&[(String, WireValue)]>) -> S3AccessParams {
    let entries = trusted
        .unwrap_or_default()
        .iter()
        .map(|(name, value)| (name.clone(), trusted_value(value)));
    S3AccessParams {
        trusted: frozen_mapping(entries),
        bucket_name: None,
        output_bucket_name: None,
    }
}

pub(super) fn region_inputs(environment: &BTreeMap<String, String>) -> RegionInputs<'_> {
    RegionInputs {
        environment_region_name: environment.get("AWS_REGION_NAME").map(String::as_str),
        environment_region: environment.get("AWS_REGION").map(String::as_str),
        session_region: None,
    }
}

// boto3's chain as the Python side sets it up: request keys sign as given, otherwise the environment pair
fn chain_identity(settings: &AwsSettings) -> Credentials {
    match (&settings.access_key_id, &settings.secret_access_key) {
        (Some(access_key_id), Some(secret_access_key)) => Credentials {
            access_key_id: access_key_id.clone(),
            secret_access_key: secret_access_key.clone(),
            session_token: settings.session_token.clone(),
        },
        (Some(_) | None, _) => Credentials {
            access_key_id: SecretString::from(ENV_ACCESS_KEY.to_owned()),
            secret_access_key: SecretString::from(ENV_KEY_VALUE.to_owned()),
            session_token: None,
        },
    }
}

pub(super) fn resolve_credentials(
    settings: &AwsSettings,
    _region: &str,
) -> Ready<Result<Credentials, CredentialError>> {
    ready(Ok(chain_identity(settings)))
}

pub(super) fn request_of<'a>(
    call: &'a Call,
    litellm: &'a mut Params,
    optional: &'a Params,
    access: &'a S3AccessParams,
    now: SystemTime,
) -> S3FilesRequest<'a> {
    S3FilesRequest {
        optional_params: optional,
        litellm_params: litellm,
        access,
        environment: &call.environment,
        region_inputs: region_inputs(&call.environment),
        now,
    }
}
