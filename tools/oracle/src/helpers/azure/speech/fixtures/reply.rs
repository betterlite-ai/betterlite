use betterlite_http::proxy::UpstreamResponse;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};

use super::super::input::Reply;

pub(crate) fn upstream(reply: &Reply) -> Result<UpstreamResponse, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in &reply.headers {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).or_config_error()?,
            HeaderValue::from_str(value).or_config_error()?,
        );
    }
    Ok(UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body.clone().into_bytes(),
    })
}
