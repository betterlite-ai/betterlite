use std::str;

use betterlite_http::proxy::UpstreamResponse;
use http::header::{HeaderMap, HeaderName};

use crate::openapi_tools::tool::OpenApiToolError;

const WWW_AUTHENTICATE: HeaderName = HeaderName::from_static("www-authenticate");

// LiteLLM's raise_for_upstream_failure: below 400 the call carries on, a 401 relays the challenge when asked
pub fn classify(
    response: &UpstreamResponse,
    relays_upstream_auth: bool,
    server: String,
) -> Result<(), OpenApiToolError> {
    if response.status < 400 {
        return Ok(());
    }
    if response.status == 401 && relays_upstream_auth {
        return Err(OpenApiToolError::UpstreamAuth {
            status: response.status,
            www_authenticate: header_text(&response.headers, &WWW_AUTHENTICATE),
            server,
        });
    }
    Err(OpenApiToolError::UpstreamStatus {
        status: response.status,
        server,
    })
}

// httpx Headers.get: every value of the header joined with ", ", decoded as UTF-8 or else Latin-1
pub fn header_text(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for value in headers.get_all(name) {
        let text = str::from_utf8(value.as_bytes()).map_or_else(
            |_| value.as_bytes().iter().copied().map(char::from).collect(),
            str::to_owned,
        );
        parts.push(text);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(", "))
}
