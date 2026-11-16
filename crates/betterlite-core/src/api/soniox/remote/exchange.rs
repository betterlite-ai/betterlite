use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::openai_base_config::Headers;
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use url::Url;

use super::error::RemoteError;
use crate::api::api_boundaries::ApiLimits;
use crate::api::soniox::prepare::decode::{SonioxReply, decode_soniox_reply};

pub const AUTHORIZATION_NAME: &str = "Authorization";
const MILLIS_PER_SECOND: u64 = 1000;

pub(super) fn endpoint_url(endpoint: &str) -> Result<Url, RemoteError> {
    Url::parse(endpoint).map_err(|source| RemoteError::Url {
        url: endpoint.to_owned(),
        source,
    })
}

// Poll and cleanup send every authenticated header, the way LiteLLM passes auth_headers through
pub(super) fn header_map(headers: &Headers) -> Result<HeaderMap, RemoteError> {
    let mut sent = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| RemoteError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| RemoteError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        sent.insert(header_name, header_value);
    }
    Ok(sent)
}

// The upload sends exactly the Authorization value LiteLLM copies out of the authenticated headers
pub(super) fn authorization_only(headers: &Headers) -> Result<HeaderMap, RemoteError> {
    let Some(value) = headers.get(AUTHORIZATION_NAME) else {
        return Err(RemoteError::MissingHeader {
            name: AUTHORIZATION_NAME,
        });
    };
    let header_value = HeaderValue::from_str(value).map_err(|source| RemoteError::HeaderValue {
        name: AUTHORIZATION_NAME.to_owned(),
        source,
    })?;
    let mut sent = HeaderMap::new();
    sent.insert(AUTHORIZATION, header_value);
    Ok(sent)
}

pub(super) fn reply(
    response: UpstreamResponse,
    limits: &ApiLimits,
) -> Result<SonioxReply, RemoteError> {
    let headers = reply_headers(&response.headers)?;
    decode_soniox_reply(i64::from(response.status), headers, &response.body, limits)
        .map_err(RemoteError::Reply)
}

fn reply_headers(source: &HeaderMap) -> Result<Headers, RemoteError> {
    let mut headers = Headers::new();
    for (name, value) in source {
        let text = value
            .to_str()
            .map_err(|_invalid| RemoteError::ReplyHeader {
                name: name.as_str().to_owned(),
            })?;
        headers.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(headers)
}

pub(super) fn interval(seconds: Decimal) -> Result<Duration, RemoteError> {
    let millis = seconds
        .checked_mul(Decimal::from(MILLIS_PER_SECOND))
        .and_then(|value| value.trunc().to_u64())
        .ok_or(RemoteError::Interval { seconds })?;
    Ok(Duration::from_millis(millis))
}

// LiteLLM interpolates the poll interval as a Python float, whose text always carries a decimal point
pub(super) fn interval_text(seconds: Decimal) -> String {
    if seconds.is_integer() {
        return format!("{seconds}.0");
    }
    seconds.normalize().to_string()
}
