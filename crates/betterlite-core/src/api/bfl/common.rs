use std::num::ParseIntError;
use std::time::Duration;

use http::header::InvalidHeaderValue;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_with::rust::double_option;
use url::Url;

pub const DEFAULT_POLLING_INTERVAL: Duration = Duration::from_millis(1500);
pub const DEFAULT_MAX_POLLING_TIME: Duration = Duration::from_secs(300);
pub const MAX_REPLY_BYTES: usize = 0x0100_0000;
pub const REGISTERED_DOMAIN: &str = "bfl.ai";

pub const IMAGE_GENERATION_MODELS: [(&str, &str); 6] = [
    ("flux-pro-1.1", "/v1/flux-pro-1.1"),
    ("flux-pro-1.1-ultra", "/v1/flux-pro-1.1-ultra"),
    ("flux-dev", "/v1/flux-dev"),
    ("flux-pro", "/v1/flux-pro"),
    ("flux-kontext-pro", "/v1/flux-kontext-pro"),
    ("flux-kontext-max", "/v1/flux-kontext-max"),
];

pub const IMAGE_EDIT_MODELS: [(&str, &str); 4] = [
    ("flux-kontext-pro", "/v1/flux-kontext-pro"),
    ("flux-kontext-max", "/v1/flux-kontext-max"),
    ("flux-pro-1.0-fill", "/v1/flux-pro-1.0-fill"),
    ("flux-pro-1.0-expand", "/v1/flux-pro-1.0-expand"),
];

#[derive(Debug, Clone, thiserror::Error)]
pub enum BflError {
    #[error(
        "BFL_API_KEY is not set. Please set it via environment variable or pass api_key parameter."
    )]
    MissingCredentials,
    #[error("the BFL api key holds a character a header cannot carry")]
    InvalidApiKey {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("Unknown BFL image generation model: {model}")]
    UnknownModel { model: String },
    #[error("Parameter {parameter} is not supported for model {model}")]
    UnsupportedParameter { parameter: String, model: String },
    #[error("the size option must be a string")]
    InvalidSizeField,
    #[error("Invalid size format: '{size}'. Expected format 'WIDTHxHEIGHT'")]
    InvalidSize {
        size: String,
        #[source]
        source: ParseIntError,
    },
    #[error("Rejected polling URL: scheme must be https")]
    PollingScheme,
    #[error("Rejected polling URL: host is not within the bfl.ai domain")]
    PollingHost,
    #[error("Rejected polling URL: not a URL")]
    PollingUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("the BFL reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the BFL reply is not a reply: {source}")]
    ReplyDecode {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the BFL reply has bytes after the reply")]
    ReplyTrailing {
        #[source]
        source: serde_json::Error,
    },
    #[error("No image URL in BFL result")]
    NoImageUrl,
}

impl BflError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::MissingCredentials => Some(401),
            Self::NoImageUrl => Some(500),
            Self::PollingScheme | Self::PollingHost | Self::PollingUrl { .. } => Some(502),
            Self::InvalidApiKey { .. }
            | Self::UnknownModel { .. }
            | Self::UnsupportedParameter { .. }
            | Self::InvalidSizeField
            | Self::InvalidSize { .. }
            | Self::ReplyTooLarge { .. }
            | Self::ReplyDecode { .. }
            | Self::ReplyTrailing { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BflSample {
    #[serde(default)]
    pub sample: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BflResource {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BflImageEntry {
    Url(String),
    Resource(BflResource),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BflResult {
    Sample(BflSample),
    List(Vec<BflImageEntry>),
    Other(IgnoredAny),
}

fn default_result() -> BflResult {
    BflResult::Other(IgnoredAny)
}

#[derive(Debug, Clone, Deserialize)]
pub struct BflReply {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub polling_url: Option<String>,
    #[serde(default, with = "double_option")]
    pub errors: Option<Option<IgnoredAny>>,
    #[serde(default = "default_result")]
    pub result: BflResult,
}

pub fn decode_bfl_reply(bytes: &[u8]) -> Result<BflReply, BflError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(BflError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES,
        });
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let reply = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| BflError::ReplyDecode { source })?;
    decoder
        .end()
        .map_err(|source| BflError::ReplyTrailing { source })?;
    Ok(reply)
}

pub fn check_bfl_poll_url(polling_url: &str) -> Result<Url, BflError> {
    let parsed = Url::parse(polling_url).map_err(|source| BflError::PollingUrl { source })?;
    if parsed.scheme() != "https" {
        return Err(BflError::PollingScheme);
    }
    let host = parsed.host_str().unwrap_or_default();
    if host != REGISTERED_DOMAIN && !host.ends_with(".bfl.ai") {
        return Err(BflError::PollingHost);
    }
    Ok(parsed)
}
