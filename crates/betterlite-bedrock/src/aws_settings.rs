use std::collections::BTreeMap;

use betterlite_config::sections::bedrock_aws_settings::AwsS3Settings;
use betterlite_config::sections::bedrock_settings::SessionTagValue;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub mod credential_cache;
pub mod error;
pub mod policy;
pub mod sources;
pub mod tls;

pub const AUTHENTICATION_PARAMS: [&str; 12] = [
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
    "aws_region_name",
    "aws_session_name",
    "aws_profile_name",
    "aws_role_name",
    "aws_web_identity_token",
    "aws_sts_endpoint",
    "aws_bedrock_runtime_endpoint",
    "aws_external_id",
    "aws_session_tags",
];

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid aws_session_tags: expected string Key and Value fields")]
    SessionTags,
    #[error("invalid AWS authentication field {field}")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid AWS authentication parameters")]
    Parameters(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwsSessionTag {
    #[serde(rename = "Key")]
    pub key: String,
    #[serde(rename = "Value")]
    pub value: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthParams {
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub aws_session_name: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub aws_external_id: Option<String>,
    #[serde(skip)]
    pub aws_session_tags: Option<Vec<AwsSessionTag>>,
}

pub fn canonicalize_tags(
    raw: Option<SessionTagValue>,
) -> Result<Option<Vec<AwsSessionTag>>, AuthError> {
    let Some(nonnull) = raw.filter(|value| !matches!(value, SessionTagValue::Null)) else {
        return Ok(None);
    };
    let SessionTagValue::List(values) = nonnull else {
        return Err(AuthError::SessionTags);
    };
    let mut tags = Vec::with_capacity(values.len());
    for entry in values {
        let SessionTagValue::Mapping(mut fields) = entry else {
            return Err(AuthError::SessionTags);
        };
        let (Some(SessionTagValue::Text(key)), Some(SessionTagValue::Text(value))) =
            (fields.shift_remove("Key"), fields.shift_remove("Value"))
        else {
            return Err(AuthError::SessionTags);
        };
        tags.push(AwsSessionTag { key, value });
    }
    tags.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(Some(tags))
}

#[derive(Deserialize)]
struct RawAuth {
    #[serde(flatten)]
    fields: AuthParams,
    aws_session_tags: Option<SessionTagValue>,
}

pub fn pop_aws_auth_params(
    parameters: &mut BTreeMap<String, Box<RawValue>>,
) -> Result<AuthParams, AuthError> {
    let mut popped = BTreeMap::new();
    for key in AUTHENTICATION_PARAMS {
        if matches!(key, "aws_region_name" | "aws_bedrock_runtime_endpoint") {
            continue;
        }
        if let Some(value) = parameters.remove(key) {
            popped.insert(key, value);
        }
    }
    for (field, value) in &popped {
        if *field != "aws_session_tags" {
            drop(
                serde_json::from_str::<Option<SecretString>>(value.get())
                    .map_err(|source| AuthError::Field { field, source })?,
            );
        }
    }
    let bytes = serde_json::to_vec(&popped)?;
    let raw: RawAuth = serde_json::from_slice(&bytes)?;
    let mut fields = raw.fields;
    fields.aws_session_tags = canonicalize_tags(raw.aws_session_tags)?;
    Ok(fields)
}

#[must_use]
pub fn s3_static_key_pair(settings: &AwsS3Settings) -> Option<(SecretString, SecretString)> {
    let access = settings.access_key_id.as_ref()?;
    let secret = settings.secret_access_key.as_ref()?;
    if access.expose_secret().is_empty() || secret.expose_secret().is_empty() {
        return None;
    }
    Some((access.clone(), secret.clone()))
}

#[must_use]
pub fn bedrock_bearer_token(
    api_key: Option<&SecretString>,
    configured: Option<&SecretString>,
) -> Option<SecretString> {
    let selected = api_key.or(configured)?;
    if selected.expose_secret().is_empty() {
        None
    } else {
        Some(selected.clone())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AssumeRoleParams {
    pub role_arn: String,
    pub role_session_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<AwsSessionTag>>,
}

#[must_use]
pub fn assume_role_parameters(
    role_arn: String,
    role_session_name: String,
    external_id: Option<String>,
    tags: Option<Vec<AwsSessionTag>>,
) -> AssumeRoleParams {
    let nonempty_tags = tags.filter(|values| !values.is_empty());
    AssumeRoleParams {
        role_arn,
        role_session_name,
        external_id,
        tags: nonempty_tags,
    }
}

mod tests;
