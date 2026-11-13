use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompresrTransportError {
    #[error("the Compresr api_base must be http or https")]
    Scheme,
    #[error("the Compresr api_base is not a valid URL")]
    InvalidApiBase,
    #[error("the Compresr api_base names no host")]
    NoHost,
    #[error("the Compresr api_base {host} is a blocked cloud-metadata host")]
    BlockedHost { host: String },
    #[error("{message}")]
    CompressionFailed { message: &'static str },
}

impl CompresrTransportError {
    // LiteLLM answers a compression failure with 502 and a config error with a ValueError
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::CompressionFailed { .. } => Some(502),
            Self::Scheme | Self::InvalidApiBase | Self::NoHost | Self::BlockedHost { .. } => None,
        }
    }
}

mod tests;
