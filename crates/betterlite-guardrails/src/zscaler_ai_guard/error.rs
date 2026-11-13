use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use serde::Serialize;
use thiserror::Error;

use super::ERROR_TYPE;

#[derive(Debug, Error)]
pub enum ZscalerAiGuardError {
    #[error("the Zscaler AI Guard policy id environment variable is not an integer: {source}")]
    PolicyId {
        #[source]
        source: betterlite_env::EnvError,
    },
    #[error("the Zscaler AI Guard send flag environment variable is invalid: {source}")]
    SendFlag {
        #[source]
        source: betterlite_env::EnvError,
    },
    #[error("the Zscaler AI Guard timeout is not a representable duration")]
    Timeout {
        #[source]
        source: std::time::TryFromFloatSecsError,
    },
    #[error("the Zscaler AI Guard header {name} has an invalid value")]
    Header {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Zscaler AI Guard metadata key {name} must be an object")]
    MetadataShape { name: &'static str },
    #[error("the Zscaler AI Guard policy id must be a number")]
    PolicyIdType,
    #[error("the Zscaler AI Guard answer has a detector response that is not an object")]
    DetectorShape,
    #[error("the Zscaler AI Guard request could not be sent: {source}")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Zscaler AI Guard answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Zscaler AI Guard answer is not valid JSON: {source}")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("Content blocked by Zscaler AI Guard: {info}")]
    Blocked { info: String },
    #[error("{reason}")]
    Http { status: StatusCode, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ZscalerErrorDetail {
    Block {
        error: String,
    },
    Service {
        error_type: &'static str,
        reason: String,
    },
}

impl ZscalerAiGuardError {
    #[must_use]
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Blocked { .. } => StatusCode::BAD_REQUEST,
            Self::Http { status, .. } => *status,
            Self::PolicyId { .. }
            | Self::SendFlag { .. }
            | Self::Timeout { .. }
            | Self::Header { .. }
            | Self::MetadataShape { .. }
            | Self::PolicyIdType
            | Self::DetectorShape
            | Self::Request { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    #[must_use]
    pub fn detail(&self) -> ZscalerErrorDetail {
        if let Self::Blocked { info } = self {
            return ZscalerErrorDetail::Block {
                error: format!("Content blocked by Zscaler AI Guard: {info}"),
            };
        }
        ZscalerErrorDetail::Service {
            error_type: ERROR_TYPE,
            reason: self.reason(),
        }
    }

    fn reason(&self) -> String {
        match self {
            Self::Http { reason, .. } => reason.clone(),
            other => other.to_string(),
        }
    }
}
