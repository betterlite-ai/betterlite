use std::fmt;

use thiserror::Error;

use crate::io_token_reservations::error::IoPreCallError;

// RoutingArgs.ttl: the minute window a tpm or rpm counter expires in
pub const RATE_LIMIT_TTL_SECONDS: u64 = 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitChannel {
    Tpm,
    Rpm,
}

impl RateLimitChannel {
    // the lowercase word the response content names the channel with
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tpm => "tpm",
            Self::Rpm => "rpm",
        }
    }

    // the uppercase word litellm's RateLimitError message names the channel with
    #[must_use]
    pub const fn message_label(self) -> &'static str {
        match self {
            Self::Tpm => "TPM",
            Self::Rpm => "RPM",
        }
    }
}

// Everything litellm.RateLimitError receives for a model rate limit, and the response it answers with
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitExceeded {
    pub channel: RateLimitChannel,
    pub limit: i64,
    pub current: i64,
    pub model_id: String,
    pub model: String,
    pub model_group: String,
    // pre_call_check sets none and async_pre_call_check sets zero, which litellm keeps apart
    pub num_retries: Option<u32>,
}

impl RateLimitExceeded {
    pub const STATUS_CODE: u16 = 429;
    pub const CODE: &'static str = "429";
    pub const ERROR_TYPE: &'static str = "throttling_error";
    pub const RETRY_AFTER_SECONDS: u64 = RATE_LIMIT_TTL_SECONDS;
    pub const LLM_PROVIDER: &'static str = "";
    // RouterErrors.user_defined_ratelimit_error.value, which the response content starts with
    pub const CONTENT_PREFIX: &'static str = "Deployment over user-defined ratelimit.";

    #[must_use]
    pub fn content(&self) -> String {
        format!(
            "{} {} limit={}. current usage={}. id={}, model_group={}",
            Self::CONTENT_PREFIX,
            self.channel.label(),
            self.limit,
            self.current,
            self.model_id,
            self.model_group
        )
    }

    // litellm.RateLimitError prefixes the message it is given with its own class name
    #[must_use]
    pub fn error_message(&self) -> String {
        format!("litellm.RateLimitError: {self}")
    }
}

impl fmt::Display for RateLimitExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Model rate limit exceeded. {} limit={}, current usage={}",
            self.channel.message_label(),
            self.limit,
            self.current
        )
    }
}

impl std::error::Error for RateLimitExceeded {}

// Everything pre_call_check can refuse the request with: the model limits, its own fields, or the reservation ahead of them
#[derive(Debug, Error)]
pub enum ModelRateLimitError {
    #[error(transparent)]
    Exceeded(#[from] RateLimitExceeded),
    #[error("deployment {field} must be an integer")]
    Limit { field: &'static str },
    #[error(transparent)]
    IoToken(#[from] IoPreCallError),
}
