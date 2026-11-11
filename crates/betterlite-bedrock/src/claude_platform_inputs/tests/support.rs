use std::collections::BTreeMap;
use std::future::ready;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::bedrock_aws_endpoints::Settings as AwsEndpoints;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::claude_platform_inputs::{
    PlatformInputs, PlatformValue, Settings as PlatformSettings, UnsupportedParams,
};
use betterlite_config::types::Config;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

use crate::aws_settings::sources::Credentials;
use crate::claude_platform_inputs::error::PlatformInputError;
use crate::claude_platform_inputs::signing::{
    PlatformSignRequest, PlatformSignature, sign_platform_request,
};
use crate::credentials::error::CredentialError;
use crate::model_routes::{ParamMap, ParamValue};

pub(super) const FIXTURE_CONFIG: &str =
    "model_list:\n- model_name: unit\n  litellm_params: {model: bedrock/unit}\n";

fn config(environment: &[(&str, &str)]) -> Config {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }.load(FIXTURE_CONFIG).unwrap()
}

pub(super) fn settings(environment: &[(&str, &str)]) -> PlatformSettings {
    config(environment).sections.claude_platform_inputs
}

pub(super) fn aws_endpoints(environment: &[(&str, &str)]) -> AwsEndpoints {
    config(environment).sections.bedrock_aws_endpoints
}

pub(super) fn empty_inputs() -> PlatformInputs {
    PlatformInputs {
        workspace_id: None,
        aws_workspace_id: None,
        anthropic_hyphen_workspace_id: None,
        anthropic_workspace_id: None,
        unsupported_params: None,
    }
}

pub(super) fn params(entries: &[(&str, ParamValue)]) -> ParamMap {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(super) fn text(value: &str) -> ParamValue {
    ParamValue::Text(value.to_owned())
}

pub(super) fn integer(value: i64) -> ParamValue {
    ParamValue::Number(serde_json::Number::from(value))
}

pub(super) fn unsupported(names: &[&str]) -> UnsupportedParams {
    UnsupportedParams::List(names.iter().map(|name| platform_value(name)).collect())
}

pub(super) fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from("AKIDFIXTURE"),
        secret_access_key: SecretString::from("fixture-secret-key"),
        session_token: None,
    }
}

pub(super) fn request_data() -> IndexMap<String, Json> {
    IndexMap::from([
        ("model".to_owned(), Json::Text("claude-fixture".to_owned())),
        (
            "max_tokens".to_owned(),
            Json::Number(serde_json::Number::from(16_i64)),
        ),
    ])
}

pub(super) fn epoch(seconds: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .unwrap()
}

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub(super) fn sign_with<F>(
    request: PlatformSignRequest<'_>,
    resolve: F,
) -> Result<PlatformSignature, PlatformInputError>
where
    F: FnOnce(&AwsSettings, &str) -> Result<Credentials, CredentialError>,
{
    runtime().block_on(sign_platform_request(request, move |settings, region| {
        ready(resolve(settings, region))
    }))
}

pub(super) fn platform_value(value: &str) -> PlatformValue {
    PlatformValue::Text(value.to_owned())
}

pub(super) fn platform_integer(value: i64) -> PlatformValue {
    PlatformValue::Number(serde_json::Number::from(value))
}
