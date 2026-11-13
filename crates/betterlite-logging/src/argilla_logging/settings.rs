use std::collections::BTreeMap;

use betterlite_env::EnvError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::credentials::ArgillaCredentials;
use crate::batch::logger::{BatchSettings, SettingsError as BatchError};

pub const API_KEY_VARIABLE: &str = "ARGILLA_API_KEY";
pub const BASE_URL_VARIABLE: &str = "ARGILLA_BASE_URL";
pub const DATASET_NAME_VARIABLE: &str = "ARGILLA_DATASET_NAME";
pub const SAMPLING_RATE_VARIABLE: &str = "ARGILLA_SAMPLING_RATE";
pub const BATCH_SIZE_VARIABLE: &str = "ARGILLA_BATCH_SIZE";
pub const SYNC_SAMPLING_RATE_VARIABLE: &str = "LANGSMITH_SAMPLING_RATE";
pub const DEFAULT_BASE_URL: &str = "http://localhost:6900/";
pub const DEFAULT_DATASET_NAME: &str = "litellm-completion";
pub const DEFAULT_SAMPLING_RATE: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadField {
    Messages,
    Response,
}

#[derive(Clone, Debug, Default)]
pub struct ConstructorArguments {
    pub api_key: Option<SecretString>,
    pub dataset_name: Option<String>,
    pub base_url: Option<String>,
}

pub struct ArgillaSettings {
    pub credentials: ArgillaCredentials,
    pub transformation: IndexMap<String, PayloadField>,
    pub sampling_rate: f64,
    pub batch: BatchSettings,
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("argilla_transformation_object is required, to log your payload to Argilla")]
    MissingTransformation,
    #[error("argilla_transformation_object must be a dictionary, to log your payload to Argilla")]
    TransformationNotDictionary,
    #[error(
        "all values in argilla_transformation_object must be a key in SUPPORTED_PAYLOAD_FIELDS, {value} is not a valid key"
    )]
    UnsupportedField { value: String },
    #[error("ARGILLA_API_KEY is not set")]
    MissingApiKey,
    #[error(transparent)]
    Batch(#[from] BatchError),
    #[error(transparent)]
    BatchSize(#[from] EnvError),
}

impl ArgillaSettings {
    pub fn from_environment(
        environment: &BTreeMap<String, String>,
        arguments: &ConstructorArguments,
        transformation_object: Option<&Json>,
    ) -> Result<Self, SettingsError> {
        Ok(Self {
            credentials: resolve_credentials(environment, arguments)?,
            transformation: validate_transformation(transformation_object)?,
            sampling_rate: digit_sampling_rate(
                environment.get(SAMPLING_RATE_VARIABLE).map(String::as_str),
            ),
            batch: batch_settings(environment)?,
            environment: environment.clone(),
        })
    }
}

fn resolve_credentials(
    environment: &BTreeMap<String, String>,
    arguments: &ConstructorArguments,
) -> Result<ArgillaCredentials, SettingsError> {
    let api_key = arguments
        .api_key
        .clone()
        .or_else(|| {
            environment
                .get(API_KEY_VARIABLE)
                .filter(|value| !value.is_empty())
                .cloned()
                .map(SecretString::from)
        })
        .ok_or(SettingsError::MissingApiKey)?;
    let base_url = arguments
        .base_url
        .clone()
        .or_else(|| {
            environment
                .get(BASE_URL_VARIABLE)
                .filter(|value| !value.is_empty())
                .cloned()
        })
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    let dataset_name = arguments
        .dataset_name
        .clone()
        .or_else(|| {
            environment
                .get(DATASET_NAME_VARIABLE)
                .filter(|value| !value.is_empty())
                .cloned()
        })
        .unwrap_or_else(|| DEFAULT_DATASET_NAME.to_owned());
    Ok(ArgillaCredentials {
        api_key,
        base_url,
        dataset_name,
    })
}

// LiteLLM's digit-only parser: any string that is not all digits falls back to 1.0, so "0.5" samples everything.
#[must_use]
pub fn digit_sampling_rate(value: Option<&str>) -> f64 {
    let Some(raw) = value else {
        return DEFAULT_SAMPLING_RATE;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return DEFAULT_SAMPLING_RATE;
    }
    trimmed.parse::<f64>().unwrap_or(DEFAULT_SAMPLING_RATE)
}

fn batch_settings(environment: &BTreeMap<String, String>) -> Result<BatchSettings, SettingsError> {
    let mut batch = BatchSettings::from_environment(environment)?;
    let requested = environment
        .get(BATCH_SIZE_VARIABLE)
        .map(|text| betterlite_env::number::<usize>(BATCH_SIZE_VARIABLE, text))
        .transpose()?
        .flatten();
    if let Some(size) = requested {
        batch.batch_size = size;
    }
    Ok(batch)
}

pub fn validate_transformation(
    transformation_object: Option<&Json>,
) -> Result<IndexMap<String, PayloadField>, SettingsError> {
    let Some(value) = transformation_object else {
        return Err(SettingsError::MissingTransformation);
    };
    let Json::Object(fields) = value else {
        return Err(SettingsError::TransformationNotDictionary);
    };
    let mut transformation = IndexMap::with_capacity(fields.len());
    for (name, field) in fields {
        let parsed = match field {
            Json::Text(text) if text == "messages" => PayloadField::Messages,
            Json::Text(text) if text == "response" => PayloadField::Response,
            other @ (Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_)) => {
                return Err(SettingsError::UnsupportedField {
                    value: serde_json::to_string(other).unwrap_or_default(),
                });
            }
        };
        transformation.insert(name.clone(), parsed);
    }
    Ok(transformation)
}
