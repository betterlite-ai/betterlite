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
use crate::zerobus::client::types::ZerobusConnection;

pub(crate) enum Endpoint<'request> {
    Token,
    Insert {
        rows: Vec<u8>,
        token: &'request SecretString,
    },
}

pub(crate) fn outbound_request(
    connection: &ZerobusConnection,
    endpoint: Endpoint<'_>,
    timeout: Duration,
) -> Result<OutboundRequest, ZerobusClientError> {
    let (url, what, authorization, body) = match endpoint {
        Endpoint::Token => (
            format!(
                "{}{TOKEN_PATH}",
                connection.workspace_url.trim_end_matches('/')
            ),
            "token",
            basic_authorization(&connection.client_id, &connection.client_secret)?,
            RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
                data: token_form(connection)?.into_bytes(),
            },
        ),
        Endpoint::Insert { rows, token } => (
            format!(
                "{}/zerobus/v1/tables/{}/insert",
                connection.server_endpoint.trim_end_matches('/'),
                connection.table_name
            ),
            "insert",
            bearer_authorization(token)?,
            RequestBody::Json(rows),
        ),
    };
    let target = parsed_url(&url, what)?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    Ok(OutboundRequest {
        method: Method::POST,
        url: target,
        query: Vec::new(),
        headers,
        body,
        deadline: timeout,
    })
}

fn parsed_url(url: &str, what: &'static str) -> Result<Url, ZerobusClientError> {
    match Url::parse(url) {
        Ok(parsed) => Ok(parsed),
        Err(source) => Err(ZerobusClientError::Url { what, source }),
    }
}

fn basic_authorization(
    client_id: &str,
    client_secret: &SecretString,
) -> Result<HeaderValue, ZerobusClientError> {
    let credentials = format!("{client_id}:{}", client_secret.expose_secret());
    let mut header = HeaderValue::try_from(format!("Basic {}", STANDARD.encode(credentials)))
        .map_err(ZerobusClientError::CredentialHeader)?;
    header.set_sensitive(true);
    Ok(header)
}

fn bearer_authorization(token: &SecretString) -> Result<HeaderValue, ZerobusClientError> {
    let mut header = HeaderValue::try_from(format!("Bearer {}", token.expose_secret()))
        .map_err(ZerobusClientError::TokenHeader)?;
    header.set_sensitive(true);
    Ok(header)
}
