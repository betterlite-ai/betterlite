use std::collections::BTreeMap;
use std::num::ParseIntError;
use std::ops::Div as _;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use rust_decimal::prelude::ToPrimitive as _;
use thiserror::Error;

const MEBIBYTE: f64 = 1_048_576.0;
const BAD_REQUEST: &str = "bad_request_error";
const INTERNAL: &str = "internal_server_error";
const CONTENT_LENGTH: &str = "content-length";

#[derive(Debug, Error)]
pub enum SizeError {
    #[error("the byte count cannot be represented as a float")]
    UnrepresentableBytes,
    #[error("the content-length value {value} is not an integer")]
    InvalidContentLength {
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("Request size is too large. Request size is {size_mb:?} MB. Max size is {limit_mb} MB")]
    RequestTooLarge { size_mb: f64, limit_mb: f64 },
    #[error(
        "Response size is too large. Response size is {size_mb:?} MB. Max size is {limit_mb} MB"
    )]
    ResponseTooLarge { size_mb: f64, limit_mb: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Request,
    Response,
}

impl SizeError {
    #[must_use]
    pub fn into_proxy_exception(self) -> ProxyException {
        let message = self.to_string();
        let (error_type, param, status) = match self {
            Self::RequestTooLarge { .. } | Self::ResponseTooLarge { .. } => {
                (BAD_REQUEST, Some(CONTENT_LENGTH), 400)
            }
            Self::UnrepresentableBytes | Self::InvalidContentLength { .. } => (INTERNAL, None, 500),
        };
        ProxyException::new(ProxyExceptionInit {
            message,
            error_type: error_type.to_owned(),
            param: param.map(str::to_owned),
            code: ExceptionCode::Status(status),
            headers: BTreeMap::new(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        })
    }
}

pub fn check_request_size(
    content_length: Option<&str>,
    body: &[u8],
    max_request_size_mb: Option<f64>,
) -> Result<(), SizeError> {
    // LiteLLM returns before reading content-length, so an unset limit must not parse the header
    if max_request_size_mb.is_none() {
        return Ok(());
    }
    if let Some(header) = content_length.filter(|value| !value.is_empty()) {
        let declared = header
            .parse::<i64>()
            .map_err(|source| SizeError::InvalidContentLength {
                value: header.to_owned(),
                source,
            })?;
        return check_size(declared, max_request_size_mb, Direction::Request);
    }
    let Ok(bytes) = i64::try_from(body.len()) else {
        return Err(SizeError::UnrepresentableBytes);
    };
    check_size(bytes, max_request_size_mb, Direction::Request)
}

pub fn check_response_size(
    body: &[u8],
    max_response_size_mb: Option<f64>,
) -> Result<(), SizeError> {
    let Ok(bytes) = i64::try_from(body.len()) else {
        return Err(SizeError::UnrepresentableBytes);
    };
    check_size(bytes, max_response_size_mb, Direction::Response)
}

fn check_size(bytes: i64, max_size_mb: Option<f64>, direction: Direction) -> Result<(), SizeError> {
    let Some(limit_mb) = max_size_mb else {
        return Ok(());
    };
    let size_mb = bytes
        .to_f64()
        .ok_or(SizeError::UnrepresentableBytes)?
        .div(MEBIBYTE);
    if size_mb > limit_mb {
        return Err(match direction {
            Direction::Request => SizeError::RequestTooLarge { size_mb, limit_mb },
            Direction::Response => SizeError::ResponseTooLarge { size_mb, limit_mb },
        });
    }
    Ok(())
}
