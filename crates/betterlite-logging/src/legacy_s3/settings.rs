use std::collections::BTreeMap;

use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use crate::s3_transport::encryption::{ServerSideEncryption, resolve_server_side_encryption};
use crate::s3_transport::option_value::OptionValue;

mod tests;

pub const BUCKET_SETTING: &str = "s3_bucket_name";
const PATH_SETTING: &str = "s3_path";
const REGION_SETTING: &str = "s3_region_name";
const ENDPOINT_SETTING: &str = "s3_endpoint_url";
const ACCESS_KEY_SETTING: &str = "s3_aws_access_key_id";
const SECRET_KEY_SETTING: &str = "s3_aws_secret_access_key";
const SESSION_TOKEN_SETTING: &str = "s3_aws_session_token";
const SSE_SETTING: &str = "s3_server_side_encryption";
const SSE_KMS_KEY_SETTING: &str = "s3_sse_kms_key_id";
const TEAM_PREFIX_SETTING: &str = "s3_use_team_prefix";
const LOG_PROMPTS_ONLY_SETTING: &str = "s3_log_prompts_only";
// litellm.enable_preview_features belongs to B024 until it loads, so the S3 block carries it
const PREVIEW_FEATURES_SETTING: &str = "enable_preview_features";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("the S3 callback setting '{name}' is not supported")]
    UnknownKey { name: String },
    #[error("the S3 setting '{name}' must be a bool")]
    Boolean {
        name: &'static str,
        #[source]
        source: betterlite_env::EnvError,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub bucket_name: Option<String>,
    pub path: String,
    pub region_name: Option<String>,
    pub endpoint_url: Option<String>,
    pub access_key_id: Option<SecretString>,
    pub secret_access_key: Option<SecretString>,
    pub session_token: Option<SecretString>,
    pub encryption: Option<ServerSideEncryption>,
    pub use_team_prefix: bool,
    pub log_prompts_only: Option<OptionValue>,
    pub preview_features: bool,
}

struct Values<'params> {
    remaining: BTreeMap<&'params str, &'params str>,
}

impl<'params> Values<'params> {
    fn new(params: &'params IndexMap<String, String>) -> Self {
        Self {
            remaining: params
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect(),
        }
    }

    fn flag(&mut self, name: &'static str) -> Result<bool, SettingsError> {
        let Some(text) = self.remaining.remove(name) else {
            return Ok(false);
        };
        Ok(betterlite_env::flag(name, text)
            .map_err(|source| SettingsError::Boolean { name, source })?
            .unwrap_or(false))
    }

    fn option(&mut self, name: &'static str) -> Option<OptionValue> {
        self.remaining
            .remove(name)
            .map(|text| OptionValue::Text(text.to_owned()))
    }

    fn finish(self) -> Result<(), SettingsError> {
        self.remaining.keys().next().map_or(Ok(()), |name| {
            Err(SettingsError::UnknownKey {
                name: (*name).to_owned(),
            })
        })
    }
}

pub fn resolve(params: &IndexMap<String, String>) -> Result<Settings, SettingsError> {
    let mut values = Values::new(params);
    let bucket_name = values.remaining.remove(BUCKET_SETTING).map(str::to_owned);
    let path = values
        .remaining
        .remove(PATH_SETTING)
        .map(str::to_owned)
        .unwrap_or_default();
    let region_name = values.remaining.remove(REGION_SETTING).map(str::to_owned);
    let endpoint_url = values.remaining.remove(ENDPOINT_SETTING).map(str::to_owned);
    let access_key_id = values
        .remaining
        .remove(ACCESS_KEY_SETTING)
        .map(SecretString::from);
    let secret_access_key = values
        .remaining
        .remove(SECRET_KEY_SETTING)
        .map(SecretString::from);
    let session_token = values
        .remaining
        .remove(SESSION_TOKEN_SETTING)
        .map(SecretString::from);
    let algorithm = values.option(SSE_SETTING);
    let kms_key = values.option(SSE_KMS_KEY_SETTING);
    let use_team_prefix = values.flag(TEAM_PREFIX_SETTING)?;
    let log_prompts_only = values.option(LOG_PROMPTS_ONLY_SETTING);
    let preview_features = values.flag(PREVIEW_FEATURES_SETTING)?;
    values.finish()?;
    Ok(Settings {
        bucket_name,
        path,
        region_name,
        endpoint_url,
        access_key_id,
        secret_access_key,
        session_token,
        encryption: resolve_server_side_encryption(algorithm.as_ref(), kms_key.as_ref()),
        use_team_prefix,
        log_prompts_only,
        preview_features,
    })
}
