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
    pub retries: i32,
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
        Ok(Self {
            retries: number::<i32>(
                "DEFAULT_REPLICATE_POLLING_RETRIES",
                retries.unwrap_or_default(),
            )?
            .unwrap_or(5),
            retry_delay_seconds: f64::from(
                number::<i32>(
                    "DEFAULT_REPLICATE_POLLING_DELAY_SECONDS",
                    retry_delay.unwrap_or_default(),
                )?
                .unwrap_or(1_i32),
            ),
            polling_delay_seconds: number::<f64>(
                "REPLICATE_POLLING_DELAY_SECONDS",
                polling_delay.unwrap_or_default(),
            )?
            .unwrap_or(0.5_f64),
            model_name_with_id_length: number::<usize>(
                "REPLICATE_MODEL_NAME_WITH_ID_LENGTH",
                model_name_with_id_length.unwrap_or_default(),
            )?
            .unwrap_or(64),
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
