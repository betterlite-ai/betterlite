use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use crate::exporter_options::options::CustomLoggerOptions;

pub const DEFAULT_API_URL: &str = "https://api.pointfive.co/api/v1/ingestion";
pub const RETRYABLE_UPLOAD_STATUS_CODES: [u16; 5] = [429, 500, 502, 503, 504];

const DEFAULT_BATCH_SIZE: usize = 1_000;
const DEFAULT_FLUSH_INTERVAL: u64 = 300;
const DEFAULT_MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_UPLOAD_RETRIES: usize = 3;

#[derive(Debug, Error)]
pub enum PointFiveParamsError {
    #[error("{field} must be greater than 0")]
    NotPositive { field: &'static str },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "RawPointFiveInitParams")]
pub struct PointFiveInitParams {
    pub api_key: Option<SecretString>,
    pub api_url: Option<String>,
    pub batch_size: usize,
    pub flush_interval: u64,
    pub max_batch_bytes: usize,
    pub max_upload_retries: usize,
    pub turn_off_message_logging: bool,
}

impl Default for PointFiveInitParams {
    fn default() -> Self {
        Self {
            api_key: None,
            api_url: None,
            batch_size: DEFAULT_BATCH_SIZE,
            flush_interval: DEFAULT_FLUSH_INTERVAL,
            max_batch_bytes: DEFAULT_MAX_BATCH_BYTES,
            max_upload_retries: DEFAULT_MAX_UPLOAD_RETRIES,
            turn_off_message_logging: false,
        }
    }
}

#[derive(Deserialize)]
struct RawPointFiveInitParams {
    api_key: Option<SecretString>,
    api_url: Option<String>,
    batch_size: Option<usize>,
    flush_interval: Option<u64>,
    max_batch_bytes: Option<usize>,
    max_upload_retries: Option<usize>,
    #[serde(flatten)]
    logger: CustomLoggerOptions,
}

impl TryFrom<RawPointFiveInitParams> for PointFiveInitParams {
    type Error = PointFiveParamsError;

    fn try_from(raw: RawPointFiveInitParams) -> Result<Self, PointFiveParamsError> {
        let batch_size = raw.batch_size.unwrap_or(DEFAULT_BATCH_SIZE);
        if batch_size == 0 {
            return Err(PointFiveParamsError::NotPositive {
                field: "batch_size",
            });
        }
        let flush_interval = raw.flush_interval.unwrap_or(DEFAULT_FLUSH_INTERVAL);
        if flush_interval == 0 {
            return Err(PointFiveParamsError::NotPositive {
                field: "flush_interval",
            });
        }
        let max_batch_bytes = raw.max_batch_bytes.unwrap_or(DEFAULT_MAX_BATCH_BYTES);
        if max_batch_bytes == 0 {
            return Err(PointFiveParamsError::NotPositive {
                field: "max_batch_bytes",
            });
        }
        let max_upload_retries = raw.max_upload_retries.unwrap_or(DEFAULT_MAX_UPLOAD_RETRIES);
        if max_upload_retries == 0 {
            return Err(PointFiveParamsError::NotPositive {
                field: "max_upload_retries",
            });
        }
        Ok(Self {
            api_key: raw.api_key,
            api_url: raw.api_url,
            batch_size,
            flush_interval,
            max_batch_bytes,
            max_upload_retries,
            turn_off_message_logging: raw.logger.turn_off_message_logging,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointFiveUploadTarget {
    pub upload_url: String,
    pub object_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointFiveUploadFailure {
    pub detail: String,
    pub retryable: bool,
}
