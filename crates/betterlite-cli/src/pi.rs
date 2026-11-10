pub mod limits;
pub mod listing;
pub mod models_json;
pub mod prepare;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

pub const PI_CONFIG_DIR_ENV: &str = "PI_CODING_AGENT_DIR";
pub const PI_PROVIDER_NAME: &str = "litellm";
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingFailure {
    Unreachable,
    Rejected,
    BadBody,
    Empty,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
pub struct ListedModel {
    pub id: String,
    #[serde(default)]
    pub source_model: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelLimits {
    pub context_window: Option<i64>,
    pub max_tokens: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait ProxyGet {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        timeout: Duration,
    ) -> Result<ProxyResponse, io::Error>;
}

// LiteLLM sends {"Authorization": f"Bearer {api_key}", **headers}, so an extra Authorization replaces the bearer value
pub(crate) fn request_headers(
    api_key: &SecretString,
    extra_headers: &[(&str, &str)],
) -> Vec<(String, SecretString)> {
    let mut headers = vec![(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", api_key.expose_secret())),
    )];
    for &(name, value) in extra_headers {
        let secret = SecretString::from(value.to_owned());
        match headers
            .iter_mut()
            .find(|(existing, _value)| existing.as_str() == name)
        {
            Some((_name, existing)) => *existing = secret,
            None => headers.push((name.to_owned(), secret)),
        }
    }
    headers
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ListingViolation {
    #[error("a model id is empty")]
    EmptyId,
    #[error("a source_model is empty")]
    EmptySourceModel,
    #[error("model ids must not map to multiple source models")]
    DuplicateMappings,
}

#[derive(Debug, Error)]
pub enum PiSyncError {
    #[error("could not list models from the proxy: {source}")]
    Unreachable {
        #[source]
        source: io::Error,
    },
    #[error("the proxy returned HTTP {status} for /v1/models; cannot list models")]
    ListingStatus { status: u16 },
    #[error("the proxy returned an unexpected /v1/models response: {source}")]
    ListingJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the proxy returned an unexpected /v1/models response: {violation}")]
    ListingInvalid { violation: ListingViolation },
    #[error("the proxy returned no models for your key")]
    NoModels,
    #[error("the proxy returned no models for your key. pi would have nothing to run")]
    NothingToRun,
    #[error("could not read {path} as a JSON object: {source}. Fix or move the file, then retry")]
    ReadModels {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read {path} as a JSON object: {source}. Fix or move the file, then retry")]
    ParseModels {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not encode {path}: {source}")]
    Encode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not write {path}: {source}")]
    WriteModels {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not locate pi's models.json: the environment has no home directory")]
    NoHome,
}

impl PiSyncError {
    #[must_use]
    pub const fn kind(&self) -> Option<ListingFailure> {
        match self {
            Self::Unreachable { .. } => Some(ListingFailure::Unreachable),
            Self::ListingStatus { status: 401 | 403 } => Some(ListingFailure::Rejected),
            Self::ListingStatus { .. } => Some(ListingFailure::Other),
            Self::ListingJson { .. } | Self::ListingInvalid { .. } => Some(ListingFailure::BadBody),
            Self::NoModels | Self::NothingToRun => Some(ListingFailure::Empty),
            Self::ReadModels { .. }
            | Self::ParseModels { .. }
            | Self::Encode { .. }
            | Self::WriteModels { .. }
            | Self::NoHome => None,
        }
    }

    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        if let Self::ListingStatus { status } = self {
            Some(*status)
        } else {
            None
        }
    }
}

mod tests;
