use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;

use super::error::TranscriptionError;
use super::params::{Params, text_param};
use super::url::{bedrock_model_id_and_region, resolve_region};
use crate::aws_settings::sources::Credentials;
use crate::signing::json_post::{JsonPost, sign_aws_json_post};
use crate::signing::request::AwsSignedRequest;

pub const SECRET_NAMES: [&str; 11] = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_REGION_NAME",
    "AWS_REGION",
    "AWS_SESSION_NAME",
    "AWS_PROFILE_NAME",
    "AWS_ROLE_NAME",
    "AWS_WEB_IDENTITY_TOKEN",
    "AWS_STS_ENDPOINT",
    "AWS_EXTERNAL_ID",
];

#[derive(Debug, Clone)]
pub enum CredentialSource {
    HostSupplied(Box<Credentials>),
    Chain(Box<AwsSettings>),
}

#[derive(Debug, Clone)]
pub struct ValidatedEnvironment {
    pub headers: Vec<(String, String)>,
    pub region: String,
    pub credentials: CredentialSource,
}

fn host_supplied(params: &Params) -> Option<Credentials> {
    let trimmed = |key: &str| {
        text_param(params, key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let access_key_id = trimmed("aws_access_key_id")?;
    let secret_access_key = trimmed("aws_secret_access_key")?;
    Some(Credentials {
        access_key_id: SecretString::from(access_key_id.to_owned()),
        secret_access_key: SecretString::from(secret_access_key.to_owned()),
        session_token: trimmed("aws_session_token")
            .map(|token| SecretString::from(token.to_owned())),
    })
}

fn chain_settings(params: &Params, env: &BTreeMap<String, String>) -> AwsSettings {
    let setting = |key: &str, name: &str| {
        text_param(params, key)
            .or_else(|| env.get(name).map(String::as_str))
            .map(str::to_owned)
    };
    let secret = |key: &str, name: &str| setting(key, name).map(SecretString::from);
    AwsSettings {
        access_key_id: secret("aws_access_key_id", "AWS_ACCESS_KEY_ID"),
        secret_access_key: secret("aws_secret_access_key", "AWS_SECRET_ACCESS_KEY"),
        session_token: secret("aws_session_token", "AWS_SESSION_TOKEN"),
        region_name: setting("aws_region_name", "AWS_REGION_NAME"),
        session_name: setting("aws_session_name", "AWS_SESSION_NAME"),
        profile_name: setting("aws_profile_name", "AWS_PROFILE_NAME"),
        role_name: setting("aws_role_name", "AWS_ROLE_NAME"),
        web_identity_token: secret("aws_web_identity_token", "AWS_WEB_IDENTITY_TOKEN"),
        sts_endpoint: setting("aws_sts_endpoint", "AWS_STS_ENDPOINT"),
        external_id: setting("aws_external_id", "AWS_EXTERNAL_ID"),
        session_tags: None,
        role_arn: None,
        web_identity_token_file: None,
        bearer_token: None,
    }
}

#[must_use]
pub fn credential_source(params: &Params, env: &BTreeMap<String, String>) -> CredentialSource {
    host_supplied(params).map_or_else(
        || CredentialSource::Chain(Box::new(chain_settings(params, env))),
        |credentials| CredentialSource::HostSupplied(Box::new(credentials)),
    )
}

#[must_use]
pub fn validate_environment(
    mut headers: Vec<(String, String)>,
    model: &str,
    params: &Params,
    env: &BTreeMap<String, String>,
) -> ValidatedEnvironment {
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
    }
    let (_, model_region) = bedrock_model_id_and_region(model);
    ValidatedEnvironment {
        headers,
        region: resolve_region(model_region, params, env),
        credentials: credential_source(params, env),
    }
}

pub fn sign_transcription(
    environment: &ValidatedEnvironment,
    url: &str,
    body: &str,
    credentials: &Credentials,
    now: SystemTime,
) -> Result<AwsSignedRequest, TranscriptionError> {
    Ok(sign_aws_json_post(JsonPost {
        service_name: "bedrock",
        region: Some(environment.region.as_str()),
        url,
        body,
        headers: &environment.headers,
        credentials,
        now,
    })?)
}
