use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use http::header::{HeaderName, HeaderValue};

use crate::api::realtime_backend_transport::error::DialError;
use crate::api::realtime_contracts::error::HeaderMap;

use super::failure::{Class, OpenFailure};

// the map a dialer hands in: B2457's backend headers carry JSON values, the health check's carry strings
#[derive(Clone, Copy)]
pub(super) enum HeaderSource<'source> {
    Backend(&'source HeaderMap),
    Probe(&'source Headers),
}

pub(super) fn dialer_headers(
    source: HeaderSource<'_>,
) -> Result<Vec<(HeaderName, HeaderValue)>, OpenFailure> {
    let mut pairs = Vec::new();
    match source {
        HeaderSource::Backend(headers) => {
            for (name, value) in headers {
                let Json::Text(text) = value else {
                    return Err(OpenFailure {
                        class: Class::ValueError,
                        status: None,
                        error: DialError::Other {
                            message: format!("the realtime backend header {name} is not a string"),
                        },
                    });
                };
                pairs.push(parse_pair(name, text)?);
            }
        }
        HeaderSource::Probe(headers) => {
            for (name, text) in headers {
                pairs.push(parse_pair(name, text)?);
            }
        }
    }
    Ok(pairs)
}

fn parse_pair(name: &str, text: &str) -> Result<(HeaderName, HeaderValue), OpenFailure> {
    let parsed_name = HeaderName::from_bytes(name.as_bytes());
    let parsed_value = HeaderValue::from_str(text);
    match (parsed_name, parsed_value) {
        (Ok(header_name), Ok(header_value)) => Ok((header_name, header_value)),
        (Err(_), _) | (_, Err(_)) => Err(OpenFailure {
            class: Class::InvalidUri,
            status: None,
            error: DialError::WebSocket {
                message: format!("the realtime backend header {name} is not valid"),
            },
        }),
    }
}
