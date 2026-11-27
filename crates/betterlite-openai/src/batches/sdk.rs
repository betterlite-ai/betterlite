use std::future::Future;
use std::time::Duration;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::batches::error::SdkError;
use crate::batches::records::{BatchCreate, BatchQuery};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Sync,
    Async,
}

// httpx.Timeout: one number for every phase, or one per phase
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Timeout {
    Seconds(f64),
    Phases(TimeoutPhases),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TimeoutPhases {
    pub connect: Option<f64>,
    pub read: Option<f64>,
    pub write: Option<f64>,
    pub pool: Option<f64>,
}

// LiteLLM reads a phase object's read phase, or ten minutes when the read phase is absent
const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;

#[derive(Debug, Error, PartialEq)]
pub enum TimeoutError {
    #[error("the timeout must be a finite, non-negative number of seconds, not {seconds}")]
    Invalid { seconds: f64 },
}

impl Timeout {
    pub fn duration(&self) -> Result<Duration, TimeoutError> {
        let seconds = match self {
            Self::Seconds(seconds) => *seconds,
            Self::Phases(phases) => phases.read.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        };
        Duration::try_from_secs_f64(seconds).map_err(|_source| TimeoutError::Invalid { seconds })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClientSettings {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<Timeout>,
    pub max_retries: Option<i64>,
    pub organization: Option<String>,
}

// The keyword arguments LiteLLM hands the SDK constructor; a None value is not passed at all
#[derive(Debug, Clone)]
pub struct SdkArguments {
    pub api_key: Option<SecretString>,
    pub base_url: Option<String>,
    pub timeout: Option<Timeout>,
    pub max_retries: Option<i64>,
    pub organization: Option<String>,
}

// One call's `_is_async`, constructor settings and optional caller-supplied client
#[derive(Debug)]
pub struct Connection<Client> {
    pub is_async: bool,
    pub settings: ClientSettings,
    pub client: Option<Client>,
}

pub trait ClientConstructor: Sync {
    type Client: BatchesSdk;

    // Ok(None) is an SDK that could not be built, for example without any API key
    fn construct(
        &self,
        arguments: &SdkArguments,
        kind: ClientKind,
    ) -> Result<Option<Self::Client>, SdkError>;
}

pub trait BatchesSdk: Send + Sync {
    type Page: Send;

    fn kind(&self) -> ClientKind;

    fn create(
        &self,
        request: &BatchCreate,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send;

    fn retrieve(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send;

    fn cancel(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send;

    fn list(
        &self,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<Self::Page, SdkError>> + Send;
}
