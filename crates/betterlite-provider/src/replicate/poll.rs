use async_trait::async_trait;
use betterlite_env::{EnvError, number};
use futures_util::stream::BoxStream;
use serde::Serialize;

use super::error::ReplicateFailure;
use super::reply::ReplicateReply;

pub mod completion;
pub mod stream;
pub mod transport;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplicatePolling {
    pub retries: i64,
    pub retry_delay_seconds: f64,
    pub polling_delay_seconds: f64,
    pub model_name_with_id_length: usize,
}

impl ReplicatePolling {
    pub fn from_env_values(
        retries: Option<&str>,
        retry_delay: Option<&str>,
        polling_delay: Option<&str>,
        model_name_with_id_length: Option<&str>,
    ) -> Result<Self, EnvError> {
        let retry_count = number::<i64>(
            "DEFAULT_REPLICATE_POLLING_RETRIES",
            retries.unwrap_or_default(),
        )?
        .unwrap_or(5);
        let retry_delay_seconds = number::<f64>(
            "DEFAULT_REPLICATE_POLLING_DELAY_SECONDS",
            retry_delay.unwrap_or_default(),
        )?
        .unwrap_or(1.0_f64);
        let polling_delay_seconds = number::<f64>(
            "REPLICATE_POLLING_DELAY_SECONDS",
            polling_delay.unwrap_or_default(),
        )?
        .unwrap_or(0.5_f64);
        let version_id_length = number::<usize>(
            "REPLICATE_MODEL_NAME_WITH_ID_LENGTH",
            model_name_with_id_length.unwrap_or_default(),
        )?
        .unwrap_or(64);
        Ok(Self {
            retries: retry_count,
            retry_delay_seconds,
            polling_delay_seconds,
            model_name_with_id_length: version_id_length,
        })
    }
}

// Strict is LiteLLM's sync stream poller and Lenient its async one; each keeps the output handling of its source
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPolicy {
    Strict,
    Lenient,
}

#[async_trait]
pub trait ReplicateSleeper: Send + Sync {
    async fn sleep(&self, seconds: f64);
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplicateChunk {
    pub output: String,
    pub status: String,
}

pub type ReplicateStream = BoxStream<'static, Result<ReplicateChunk, ReplicateFailure>>;

pub enum ReplicateCompletion {
    Reply(ReplicateReply),
    Stream(ReplicateStream),
}

mod tests;
