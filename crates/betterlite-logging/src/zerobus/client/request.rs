use std::str;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::zerobus::client::endpoint::{TOKEN_PATH, token_form};
use crate::zerobus::client::error::ZerobusClientError;
use crate::zerobus::client::types::{
    RETRYABLE_INGEST_STATUS_CODES, ZerobusConnection, ZerobusIngestFailure,
};

pub fn token_request(
    connection: &ZerobusConnection,
    timeout: Duration,
) -> Result<OutboundRequest, ZerobusClientError> {
    let url = Url::parse(&format!(
        "{}{TOKEN_PATH}",
        connection.workspace_url.trim_end_matches('/')
    ))
    .map_err(|source| ZerobusClientError::Url {
        what: "token",
        source,
    })?;
    let credentials = format!(
        "{}:{}",
        connection.client_id,
        connection.client_secret.expose_secret()
    );
    let mut authorization =
        HeaderValue::try_from(format!("Basic {}", STANDARD.encode(credentials)))
            .map_err(ZerobusClientError::CredentialHeader)?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Bytes {
            content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
            data: token_form(connection)?.into_bytes(),
        },
        deadline: timeout,
    })
}

pub fn insert_request(
    connection: &ZerobusConnection,
    rows: Vec<u8>,
    token: &SecretString,
    timeout: Duration,
) -> Result<OutboundRequest, ZerobusClientError> {
    let url = Url::parse(&format!(
        "{}/zerobus/v1/tables/{}/insert",
        connection.server_endpoint.trim_end_matches('/'),
        connection.table_name
    ))
    .map_err(|source| ZerobusClientError::Url {
        what: "insert",
        source,
    })?;
    let mut authorization = HeaderValue::try_from(format!("Bearer {}", token.expose_secret()))
        .map_err(ZerobusClientError::TokenHeader)?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(rows),
        deadline: timeout,
    })
}

#[must_use]
pub fn status_failure(what: &str, status: u16, body: &[u8]) -> ZerobusIngestFailure {
    ZerobusIngestFailure {
        detail: format!("{what} returned {status}: {}", body_text(body))
            .chars()
            .take(500)
            .collect(),
        retryable: RETRYABLE_INGEST_STATUS_CODES.contains(&status),
    }
}

// A rejected body that is not UTF-8 keeps the text up to its first invalid byte
fn body_text(body: &[u8]) -> &str {
    match str::from_utf8(body) {
        Ok(text) => text,
        Err(invalid) => str::from_utf8(body.get(..invalid.valid_up_to()).unwrap_or_default())
            .unwrap_or_default(),
    }
}
