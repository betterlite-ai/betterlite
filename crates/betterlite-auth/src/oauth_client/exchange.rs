use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::oauth_client::OAUTH_REQUEST_TIMEOUT;
use crate::oauth_client::OAuthClient;
use crate::oauth_client::document::{
    OAuthDiscoveryDocument, OAuthTokenResponse, OAuthUserInfo, decode_oauth_provider_document,
};
use crate::oauth_client::error::{
    OAuthClientError, OAuthEndpoint, OAuthProviderFailure, endpoint_url, require_secure,
};

const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

pub struct CallbackRequest<'a> {
    pub url: &'a str,
    pub sso_state_cookie: Option<&'a str>,
}

pub struct ExchangeRequest<'a> {
    pub callback: CallbackRequest<'a>,
    pub redirect_uri: Option<&'a str>,
    pub include_client_id: bool,
}

pub struct OAuthExchange {
    pub tokens: OAuthTokenResponse,
    pub userinfo: OAuthUserInfo,
}

struct TokenParameters<'a> {
    code: &'a str,
    redirect_uri: &'a str,
    include_client_id: bool,
}

pub async fn exchange(
    client: &OAuthClient,
    transport: &ProxiedClient,
    discovery: &OAuthDiscoveryDocument,
    request: &ExchangeRequest<'_>,
) -> Result<OAuthExchange, OAuthClientError> {
    let callback = Url::parse(request.callback.url).map_err(OAuthClientError::CallbackUrl)?;
    let pairs: Vec<(String, String)> = callback.query_pairs().into_owned().collect();
    let Some(code) = last_query_value(&pairs, "code").filter(|value| !value.is_empty()) else {
        return Err(OAuthClientError::MissingCode);
    };
    check_state(&pairs, request.callback.sso_state_cookie)?;
    if let Some(error) = last_non_blank_query_value(&pairs, "error") {
        return Err(OAuthClientError::Provider {
            error: error.to_owned(),
            description: last_non_blank_query_value(&pairs, "error_description").map(str::to_owned),
        });
    }
    let redirect_uri = resolved_redirect_uri(client, request, &callback);
    let parameters = TokenParameters {
        code,
        redirect_uri: &redirect_uri,
        include_client_id: request.include_client_id,
    };
    let tokens = request_tokens(client, transport, &discovery.token_endpoint, &parameters).await?;
    let userinfo = fetch_userinfo(
        client,
        transport,
        &discovery.userinfo_endpoint,
        &tokens.access_token,
    )
    .await?;
    Ok(OAuthExchange { tokens, userinfo })
}

fn check_state(pairs: &[(String, String)], cookie: Option<&str>) -> Result<(), OAuthClientError> {
    let Some(state) = last_query_value(pairs, "state") else {
        return Ok(());
    };
    let Some(cookie) = cookie else {
        return Ok(());
    };
    if cookie == state {
        return Ok(());
    }
    Err(OAuthClientError::InvalidState)
}

fn resolved_redirect_uri(
    client: &OAuthClient,
    request: &ExchangeRequest<'_>,
    callback: &Url,
) -> String {
    let configured = request
        .redirect_uri
        .filter(|value| !value.is_empty())
        .or(client
            .redirect_uri
            .as_deref()
            .filter(|value| !value.is_empty()));
    if let Some(value) = configured {
        return value.to_owned();
    }
    let origin = callback.origin().ascii_serialization();
    let path = callback.path();
    format!("{origin}{path}")
}

async fn request_tokens(
    client: &OAuthClient,
    transport: &ProxiedClient,
    endpoint: &str,
    parameters: &TokenParameters<'_>,
) -> Result<OAuthTokenResponse, OAuthClientError> {
    let url = endpoint_url(endpoint, OAuthEndpoint::Token)?;
    require_secure(client, &url, OAuthEndpoint::Token)?;
    let headers = token_headers(client)?;
    let body = token_body(client, parameters);
    let response = transport
        .send_buffered(OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static(FORM_CONTENT_TYPE),
                data: body.into_bytes(),
            },
            deadline: OAUTH_REQUEST_TIMEOUT,
        })
        .await
        .map_err(|source| OAuthClientError::Token(OAuthProviderFailure::Transport(source)))?;
    require_success(response.status).map_err(OAuthClientError::Token)?;
    let tokens: OAuthTokenResponse = decode_oauth_provider_document(&response.body)
        .map_err(|source| OAuthClientError::Token(OAuthProviderFailure::Document(source)))?;
    if tokens.access_token.expose_secret().is_empty() {
        return Err(OAuthClientError::Token(
            OAuthProviderFailure::MissingAccessToken,
        ));
    }
    Ok(tokens)
}

fn token_headers(client: &OAuthClient) -> Result<HeaderMap, OAuthClientError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(FORM_CONTENT_TYPE));
    let client_id = client.client_id.as_str();
    let secret = client.client_secret.expose_secret();
    let encoded = STANDARD.encode(format!("{client_id}:{secret}"));
    let credentials = format!("Basic {encoded}");
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&credentials).map_err(OAuthClientError::Credentials)?,
    );
    overlay_additional_headers(&mut headers, client);
    Ok(headers)
}

async fn fetch_userinfo(
    client: &OAuthClient,
    transport: &ProxiedClient,
    endpoint: &str,
    access_token: &SecretString,
) -> Result<OAuthUserInfo, OAuthClientError> {
    let url = endpoint_url(endpoint, OAuthEndpoint::Userinfo)?;
    require_secure(client, &url, OAuthEndpoint::Userinfo)?;
    let mut headers = HeaderMap::new();
    let token = access_token.expose_secret();
    let bearer = format!("Bearer {token}");
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&bearer).map_err(OAuthClientError::Credentials)?,
    );
    overlay_additional_headers(&mut headers, client);
    let response = transport
        .send_buffered(OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: OAUTH_REQUEST_TIMEOUT,
        })
        .await
        .map_err(|source| OAuthClientError::Userinfo(OAuthProviderFailure::Transport(source)))?;
    require_success(response.status).map_err(OAuthClientError::Userinfo)?;
    decode_oauth_provider_document(&response.body)
        .map_err(|source| OAuthClientError::Userinfo(OAuthProviderFailure::Document(source)))
}

fn overlay_additional_headers(headers: &mut HeaderMap, client: &OAuthClient) {
    for (name, value) in &client.additional_headers {
        headers.insert(name.clone(), value.clone());
    }
}

fn require_success(status: u16) -> Result<(), OAuthProviderFailure> {
    if (200..=299).contains(&status) {
        return Ok(());
    }
    Err(OAuthProviderFailure::Status { status })
}

fn token_body(client: &OAuthClient, parameters: &TokenParameters<'_>) -> String {
    let mut body = url::form_urlencoded::Serializer::new(String::new());
    body.append_pair("grant_type", "authorization_code");
    if parameters.include_client_id {
        body.append_pair("client_id", &client.client_id);
    }
    body.append_pair("code", parameters.code);
    body.append_pair("redirect_uri", parameters.redirect_uri);
    body.finish()
}

fn last_query_value<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn last_non_blank_query_value<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .rev()
        .find(|(key, value)| key == name && !value.is_empty())
        .map(|(_, value)| value.as_str())
}
