use std::collections::BTreeMap;
use std::num::TryFromIntError;

use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::batch::logger::{BatchSettings, SettingsError as BatchSettingsError};

pub const DEFAULT_API_URL: &str = "https://cloud.getliteral.ai";
pub const GRAPHQL_PATH: &str = "/api/graphql";
pub const CLIENT_NAME: &str = "litellm";
pub const API_KEY_HEADER: &str = "x-api-key";
pub const CLIENT_NAME_HEADER: &str = "x-client-name";
pub const ENVIRONMENT_HEADER: &str = "x-env";

// The LiteralAILogger constructor arguments the proxy never passes but a direct caller can.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub api_key: Option<SecretString>,
    pub api_url: Option<String>,
    pub environment: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub api_url: String,
    pub api_key: Option<SecretString>,
    pub environment: Option<String>,
    pub batch: BatchSettings,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(transparent)]
    Environment(#[from] betterlite_env::EnvError),
    #[error(transparent)]
    Batch(#[from] BatchSettingsError),
    #[error("environment variable LITERAL_BATCH_SIZE is outside the range a batch size can hold")]
    BatchSize(#[source] TryFromIntError),
    #[error("the Literal AI header value could not be built")]
    Header(#[source] InvalidHeaderValue),
}

impl Settings {
    // LiteralAILogger.__init__: an environment URL wins over the constructor's, a constructor key wins over the environment's.
    pub fn from_environment(
        environment: &BTreeMap<String, String>,
        overrides: Overrides,
    ) -> Result<Self, SettingsError> {
        let api_url = environment
            .get("LITERAL_API_URL")
            .filter(|url| !url.is_empty())
            .cloned()
            .or(overrides.api_url)
            .unwrap_or_else(|| DEFAULT_API_URL.to_owned());
        let api_key = overrides.api_key.or_else(|| {
            environment
                .get("LITERAL_API_KEY")
                .filter(|key| !key.is_empty())
                .cloned()
                .map(SecretString::from)
        });
        // CustomBatchLogger's DEFAULT_BATCH_SIZE and DEFAULT_FLUSH_INTERVAL_SECONDS apply unless LITERAL_BATCH_SIZE overrides the size.
        let mut batch = BatchSettings::from_environment(environment)?;
        let literal_batch_size = environment
            .get("LITERAL_BATCH_SIZE")
            .map(|text| betterlite_env::number::<i64>("LITERAL_BATCH_SIZE", text))
            .transpose()?
            .flatten();
        // int("0") is falsy in CustomBatchLogger's `batch_size or DEFAULT_BATCH_SIZE`, so only a nonzero value overrides.
        if let Some(value) = literal_batch_size.filter(|value| *value != 0) {
            batch.batch_size = usize::try_from(value.max(0)).map_err(SettingsError::BatchSize)?;
        }
        Ok(Self {
            api_url,
            api_key,
            environment: overrides.environment,
            batch,
        })
    }

    pub fn headers(&self) -> Result<HeaderMap, SettingsError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            HeaderName::from_static(CLIENT_NAME_HEADER),
            HeaderValue::from_static(CLIENT_NAME),
        );
        if let Some(key) = &self.api_key {
            let mut value =
                HeaderValue::try_from(key.expose_secret()).map_err(SettingsError::Header)?;
            value.set_sensitive(true);
            headers.insert(HeaderName::from_static(API_KEY_HEADER), value);
        }
        if let Some(environment) = &self.environment {
            let value =
                HeaderValue::try_from(environment.as_str()).map_err(SettingsError::Header)?;
            headers.insert(HeaderName::from_static(ENVIRONMENT_HEADER), value);
        }
        Ok(headers)
    }
}
