use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use http::header::{HeaderName, HeaderValue};

use crate::api::realtime_backend_transport::error::DialError;
use crate::api::realtime_contracts::error::HeaderMap;

use super::failure::{Class, OpenFailure};

// B2457's backend headers carry JSON values; a text value is the header text and any other value names the header
pub(super) fn backend_headers(
    headers: &HeaderMap,
) -> Result<Vec<(HeaderName, HeaderValue)>, OpenFailure> {
    let mut pairs = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        let Json::Text(text) = value else {
            return Err(OpenFailure::new(
                Class::ValueError,
                DialError::Other {
                    message: format!("the realtime backend header {name} is not a string"),
                },
            ));
        };
        pairs.push((header_name(name)?, header_value(name, text)?));
    }
    Ok(pairs)
}

// the health check's headers are already strings
pub(super) fn probe_headers(
    headers: &Headers,
) -> Result<Vec<(HeaderName, HeaderValue)>, OpenFailure> {
    let mut pairs = Vec::with_capacity(headers.len());
    for (name, text) in headers {
        pairs.push((header_name(name)?, header_value(name, text)?));
    }
    Ok(pairs)
}

fn header_name(name: &str) -> Result<HeaderName, OpenFailure> {
    HeaderName::from_bytes(name.as_bytes()).map_err(|_error| {
        OpenFailure::new(
            Class::InvalidUri,
            DialError::WebSocket {
                message: format!("the realtime backend header {name} is not valid"),
            },
        )
    })
}

fn header_value(name: &str, text: &str) -> Result<HeaderValue, OpenFailure> {
    HeaderValue::from_str(text).map_err(|_error| {
        OpenFailure::new(
            Class::InvalidUri,
            DialError::WebSocket {
                message: format!("the realtime backend header {name} is not valid"),
            },
        )
    })
}
