use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_env::EnvError;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::batch::logger::{BatchSettings, DEFAULT_MAX_QUEUE_SIZE};

use super::credentials::CredentialParams;
use super::encryption::{Encryption, EncryptionError};

pub const DEFAULT_SQS_BATCH_SIZE: usize = 512;
pub const DEFAULT_FLUSH_INTERVAL_SECONDS: u64 = 10;
pub const DEFAULT_STRIP_DEPTH: i64 = 10;
pub const BATCH_SIZE_VARIABLE: &str = "DEFAULT_SQS_BATCH_SIZE";
pub const FLUSH_INTERVAL_VARIABLE: &str = "DEFAULT_SQS_FLUSH_INTERVAL_SECONDS";
pub const STRIP_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER";

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("the SQS callback setting '{name}' is not supported")]
    UnknownKey { name: String },
    #[error("the SQS setting '{name}' must be a bool")]
    Boolean {
        name: &'static str,
        #[source]
        source: EnvError,
    },
    #[error("the SQS setting '{name}' must be an integer")]
    Integer {
        name: &'static str,
        #[source]
        source: EnvError,
    },
    #[error(
        "the SQS setting '{name}' refers to environment variable '{variable}', which is not set"
    )]
    MissingEnvironment {
        name: &'static str,
        variable: String,
    },
    #[error(transparent)]
    Environment(#[from] EnvError),
    #[error("sqs_app_encryption_key_b64 is required when encryption is enabled.")]
    MissingEncryptionKey,
    #[error(transparent)]
    Encryption(#[from] EncryptionError),
}

pub struct Connection {
    pub queue_url: Option<String>,
    pub region: Option<String>,
}

pub struct Settings {
    pub connection: Connection,
    pub credentials: CredentialParams,
    pub strip_base64_files: bool,
    pub strip_max_depth: i64,
    pub encryption: Option<Encryption>,
    pub batch: BatchSettings,
}

#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub queue_url: Option<String>,
    pub region: Option<String>,
    pub credentials: CredentialParams,
    pub strip_base64_files: Option<bool>,
    pub encryption: Option<EncryptionOverrides>,
    pub flush_interval_seconds: Option<u64>,
    pub batch_size: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct EncryptionOverrides {
    pub key_b64: SecretString,
    pub aad: Option<String>,
}

struct Values<'params> {
    remaining: BTreeMap<&'params str, &'params str>,
    environment: &'params dyn EnvironmentLookup,
}

impl<'params> Values<'params> {
    fn new(
        params: &'params IndexMap<String, String>,
        environment: &'params dyn EnvironmentLookup,
    ) -> Self {
        Self {
            remaining: params
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect(),
            environment,
        }
    }

    fn expand(&self, name: &'static str, value: &str) -> Result<String, SettingsError> {
        value.strip_prefix("os.environ/").map_or_else(
            || Ok(value.to_owned()),
            |variable| {
                self.environment
                    .lookup(variable)
                    .ok_or_else(|| SettingsError::MissingEnvironment {
                        name,
                        variable: variable.to_owned(),
                    })
            },
        )
    }

    fn text(&mut self, name: &'static str) -> Result<Option<String>, SettingsError> {
        self.remaining
            .remove(name)
            .map_or_else(|| Ok(None), |value| self.expand(name, value).map(Some))
    }

    fn flag(&mut self, name: &'static str) -> Result<Option<bool>, SettingsError> {
        let extracted = self.text(name)?;
        extracted.map_or(Ok(None), |text| {
            betterlite_env::flag(name, &text)
                .map_err(|source| SettingsError::Boolean { name, source })
        })
    }

    fn finish(self) -> Result<(), SettingsError> {
        self.remaining.keys().next().map_or(Ok(()), |name| {
            Err(SettingsError::UnknownKey {
                name: (*name).to_owned(),
            })
        })
    }
}

fn environment_number<T>(
    environment: &dyn EnvironmentLookup,
    name: &str,
) -> Result<Option<T>, SettingsError>
where
    T: FromStr,
{
    match environment.lookup(name) {
        Some(text) => Ok(betterlite_env::number::<T>(name, &text)?),
        None => Ok(None),
    }
}

fn read_connection(
    values: &mut Values<'_>,
    overrides: &mut Overrides,
) -> Result<Connection, SettingsError> {
    Ok(Connection {
        queue_url: values
            .text("sqs_queue_url")?
            .or_else(|| overrides.queue_url.take()),
        region: values
            .text("sqs_region_name")?
            .or_else(|| overrides.region.take()),
    })
}

fn read_credentials(
    values: &mut Values<'_>,
    overrides: &Overrides,
    region: Option<String>,
) -> Result<CredentialParams, SettingsError> {
    let mut params = overrides.credentials.clone();
    for (name, slot) in [
        ("sqs_aws_access_key_id", &mut params.access_key_id),
        ("sqs_aws_secret_access_key", &mut params.secret_access_key),
        ("sqs_aws_session_token", &mut params.session_token),
        ("sqs_aws_web_identity_token", &mut params.web_identity_token),
    ] {
        if let Some(value) = values.text(name)? {
            *slot = Some(SecretString::from(value));
        }
    }
    for (name, slot) in [
        ("sqs_aws_session_name", &mut params.session_name),
        ("sqs_aws_profile_name", &mut params.profile_name),
        ("sqs_aws_role_name", &mut params.role_name),
        ("sqs_aws_sts_endpoint", &mut params.sts_endpoint),
    ] {
        if let Some(value) = values.text(name)? {
            *slot = Some(value);
        }
    }
    params.region_name = region;
    Ok(params)
}

fn read_encryption(
    values: &mut Values<'_>,
    supplied: Option<EncryptionOverrides>,
) -> Result<Option<Encryption>, SettingsError> {
    let enabled = values
        .flag("sqs_aws_use_application_level_encryption")?
        .unwrap_or_else(|| supplied.is_some());
    let key_b64 = values
        .text("sqs_app_encryption_key_b64")?
        .map(SecretString::from)
        .or_else(|| supplied.as_ref().map(|settings| settings.key_b64.clone()));
    let aad = values.text("sqs_app_encryption_aad")?.or_else(|| {
        let supplied_settings = supplied?;
        supplied_settings.aad
    });
    if enabled {
        let key = key_b64.ok_or(SettingsError::MissingEncryptionKey)?;
        return Ok(Some(Encryption::from_base64(&key, aad)?));
    }
    Ok(None)
}

fn read_batch(
    overrides: &mut Overrides,
    environment: &dyn EnvironmentLookup,
) -> Result<BatchSettings, SettingsError> {
    let default_batch_size =
        environment_number(environment, BATCH_SIZE_VARIABLE)?.unwrap_or(DEFAULT_SQS_BATCH_SIZE);
    let default_flush_seconds = environment_number(environment, FLUSH_INTERVAL_VARIABLE)?
        .unwrap_or(DEFAULT_FLUSH_INTERVAL_SECONDS);
    let batch_size = overrides.batch_size.take().unwrap_or(default_batch_size);
    let flush_seconds = overrides
        .flush_interval_seconds
        .take()
        .unwrap_or(default_flush_seconds);
    Ok(BatchSettings {
        batch_size,
        flush_interval: Duration::from_secs(flush_seconds),
        max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
    })
}

impl Settings {
    pub fn resolve(
        params: &IndexMap<String, String>,
        mut overrides: Overrides,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, SettingsError> {
        let mut values = Values::new(params, environment);
        let connection = read_connection(&mut values, &mut overrides)?;
        let credentials = read_credentials(&mut values, &overrides, connection.region.clone())?;
        let encryption = read_encryption(&mut values, overrides.encryption.take())?;
        let strip_base64_files = values
            .flag("sqs_strip_base64_files")?
            .or_else(|| overrides.strip_base64_files.take())
            .unwrap_or(false);
        let strip_max_depth =
            environment_number(environment, STRIP_DEPTH_VARIABLE)?.unwrap_or(DEFAULT_STRIP_DEPTH);
        values.finish()?;
        let batch = read_batch(&mut overrides, environment)?;
        Ok(Self {
            connection,
            credentials,
            strip_base64_files,
            strip_max_depth,
            encryption,
            batch,
        })
    }
}

mod tests;
