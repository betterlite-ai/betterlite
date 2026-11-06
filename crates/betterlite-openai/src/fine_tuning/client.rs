use std::fmt;

use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;

use super::error::FineTuneError;
use super::job::FineTuneCreate;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SdkError {
    pub status: u16,
    pub error_type: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BuildError {
    #[error("the SDK client constructor rejected the argument {name}")]
    UnexpectedArgument { name: String },
}

pub trait JobsApi {
    fn is_async(&self) -> bool;
    fn create(&self, data: &FineTuneCreate) -> Result<Vec<u8>, SdkError>;
    fn list(&self, after: Option<&str>, limit: Option<i64>) -> Result<Vec<u8>, SdkError>;
    fn retrieve(&self, job_id: &str) -> Result<Vec<u8>, SdkError>;
    fn cancel(&self, job_id: &str) -> Result<Vec<u8>, SdkError>;
}

pub trait ClientBuilder {
    fn build(
        &self,
        is_async: bool,
        args: &ClientArgs,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError>;
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ClientSettings {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub timeout: Option<f64>,
    pub max_retries: Option<i64>,
    pub organization: Option<String>,
}

#[derive(Default)]
pub struct ClientArgs {
    pub api_key: Option<SecretString>,
    pub base_url: Option<String>,
    pub api_version: Option<String>,
    pub timeout: Option<f64>,
    pub max_retries: Option<i64>,
    pub organization: Option<String>,
    pub litellm_params: Option<Map<String, Value>>,
}

impl fmt::Debug for ClientArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientArgs")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

pub struct Connection<'builder> {
    pub settings: ClientSettings,
    pub litellm_params: Option<Map<String, Value>>,
    pub client: Option<Box<dyn JobsApi>>,
    pub is_async: bool,
    pub builder: &'builder dyn ClientBuilder,
}

impl Connection<'_> {
    pub fn get_openai_client(self) -> Result<Option<Box<dyn JobsApi>>, FineTuneError> {
        if self.client.is_some() {
            return Ok(self.client);
        }
        let args = ClientArgs {
            api_key: self.settings.api_key,
            base_url: self.settings.api_base,
            api_version: self.settings.api_version,
            timeout: self.settings.timeout,
            max_retries: self.settings.max_retries,
            organization: self.settings.organization,
            litellm_params: self.litellm_params,
        };
        self.builder
            .build(self.is_async, &args)
            .map_err(|source| FineTuneError::Build { source })
    }

    pub fn open(self) -> Result<Box<dyn JobsApi>, FineTuneError> {
        let is_async = self.is_async;
        let client = self
            .get_openai_client()?
            .ok_or(FineTuneError::ClientMissing)?;
        if is_async && !client.is_async() {
            return Err(FineTuneError::NotAsyncClient);
        }
        Ok(client)
    }
}
