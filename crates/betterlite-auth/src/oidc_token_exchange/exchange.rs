use std::time::Duration;

use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::{AUTHORIZATION, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::Url;

use crate::oidc_token_exchange::decode::{
    MAX_OIDC_RESPONSE_BYTES, OidcResponseError, ProviderClaims, TokenResponse, UserinfoDocument,
    decode_oidc_token_response, decode_oidc_userinfo,
};

use self::claims::{fallback_claims, merge_claims};
use self::request::{additional_headers, authorization_header, token_form, token_headers};

mod claims;
mod request;

// LiteLLM POSTs the code exchange with `timeout=30.0` and leaves the userinfo GET on httpx's default.
pub const TOKEN_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);
pub const USERINFO_TIMEOUT: Duration = Duration::from_secs(600);
pub const TOKEN_EXCHANGE_PARAM: &str = "token_exchange";
pub const USERINFO_PARAM: &str = "userinfo";

const ID_TOKEN_FIELD: &str = "id_token";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

// the fields B1610 validates, passed as they are so this module reads no settings
#[derive(Debug)]
pub struct TokenExchangeRequest {
    pub authorization_code: String,
    pub code_verifier: SecretString,
    pub client_id: String,
    pub client_secret: Option<SecretString>,
    pub token_endpoint: String,
    pub userinfo_endpoint: Option<String>,
    pub include_client_id: bool,
    pub redirect_url: Option<String>,
    pub additional_headers: Vec<(String, String)>,
}

#[derive(Debug, Error)]
pub enum OidcTokenExchangeError {
    #[error("the SSO token endpoint URL is not valid")]
    TokenEndpointUrl(#[source] url::ParseError),
    #[error("the additional SSO header name '{name}' is not a valid header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the additional SSO header '{name}' carries a byte a header value cannot hold")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the SSO token endpoint could not be reached")]
    TokenTransport(#[source] TransportError),
    #[error("the SSO token endpoint answered HTTP {status}")]
    TokenStatus { status: u16 },
    #[error("the SSO token endpoint answered with a response that fails validation")]
    TokenResponse(#[source] OidcResponseError),
    #[error("the SSO userinfo endpoint URL is not valid")]
    UserinfoUrl(#[source] url::ParseError),
    #[error("the SSO userinfo request failed")]
    UserinfoTransport(#[source] TransportError),
    #[error(
        "the SSO userinfo response gives no identity claims and the token response carries no usable id_token"
    )]
    UserinfoUnavailable,
    #[error("the SSO id_token is not a decodable JWT")]
    IdTokenInvalid,
    #[error("the SSO id_token payload cannot be read as JSON claims")]
    IdTokenClaims(#[source] serde_json::Error),
    #[error("the SSO userinfo response is not a JSON object")]
    UserinfoNotAnObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OidcTokenExchangeErrorResponse {
    pub status: u16,
    pub error_type: ProxyErrorTypes,
    pub param: Option<&'static str>,
    pub code: &'static str,
}

impl OidcTokenExchangeError {
    // LiteLLM raises the token exchange as 401 auth_error/token_exchange, the identity fallback as userinfo, and crashes on a non-object userinfo body
    #[must_use]
    pub const fn response(&self) -> OidcTokenExchangeErrorResponse {
        match self {
            Self::TokenEndpointUrl(_)
            | Self::HeaderName { .. }
            | Self::HeaderValue { .. }
            | Self::TokenTransport(_)
            | Self::TokenStatus { .. }
            | Self::TokenResponse(_) => OidcTokenExchangeErrorResponse {
                status: 401,
                error_type: ProxyErrorTypes::AuthError,
                param: Some(TOKEN_EXCHANGE_PARAM),
                code: "401",
            },
            Self::UserinfoUrl(_)
            | Self::UserinfoTransport(_)
            | Self::UserinfoUnavailable
            | Self::IdTokenInvalid
            | Self::IdTokenClaims(_) => OidcTokenExchangeErrorResponse {
                status: 401,
                error_type: ProxyErrorTypes::AuthError,
                param: Some(USERINFO_PARAM),
                code: "401",
            },
            Self::UserinfoNotAnObject => OidcTokenExchangeErrorResponse {
                status: 500,
                error_type: ProxyErrorTypes::InternalServerError,
                param: None,
                code: "500",
            },
        }
    }
}

// LiteLLM's _pkce_token_exchange plus _get_pkce_userinfo: the userinfo claims win, the token response keeps the bearer fields
pub async fn exchange_pkce_code(
    client: &ProxiedClient,
    request: &TokenExchangeRequest,
) -> Result<ProviderClaims, OidcTokenExchangeError> {
    let token_response = post_token_request(client, request).await?;
    let userinfo = get_userinfo(client, request, &token_response.access_token).await;
    let identity = match userinfo {
        Some(UserinfoDocument::Claims(claims)) => claims,
        Some(UserinfoDocument::NotAnObject) => {
            return Err(OidcTokenExchangeError::UserinfoNotAnObject);
        }
        Some(UserinfoDocument::Empty) | None => fallback_claims(&token_response)?,
    };
    Ok(merge_claims(token_response.claims, identity))
}

async fn post_token_request(
    client: &ProxiedClient,
    request: &TokenExchangeRequest,
) -> Result<TokenResponse, OidcTokenExchangeError> {
    let url =
        Url::parse(&request.token_endpoint).map_err(OidcTokenExchangeError::TokenEndpointUrl)?;
    let outbound = OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers: token_headers(request)?,
        body: RequestBody::Bytes {
            content_type: HeaderValue::from_static(FORM_CONTENT_TYPE),
            data: token_form(request).into_bytes(),
        },
        deadline: TOKEN_EXCHANGE_TIMEOUT,
    };
    let response = client
        .send_following(outbound)
        .await
        .map_err(OidcTokenExchangeError::TokenTransport)?
        .buffered(MAX_OIDC_RESPONSE_BYTES)
        .await
        .map_err(OidcTokenExchangeError::TokenTransport)?;
    validated_token_response(&response)
}

fn validated_token_response(
    response: &UpstreamResponse,
) -> Result<TokenResponse, OidcTokenExchangeError> {
    if response.status != 200 {
        tracing::error!(
            status = response.status,
            "the SSO token endpoint failed the code exchange"
        );
        return Err(OidcTokenExchangeError::TokenStatus {
            status: response.status,
        });
    }
    let token_response = decode_oidc_token_response(&response.body).map_err(|source| {
        tracing::error!(error = %source, "the SSO token endpoint response failed validation");
        OidcTokenExchangeError::TokenResponse(source)
    })?;
    tracing::debug!(
        id_token_present = token_response.claims.contains_key(ID_TOKEN_FIELD),
        "the SSO token endpoint answered the code exchange"
    );
    Ok(token_response)
}

async fn get_userinfo(
    client: &ProxiedClient,
    request: &TokenExchangeRequest,
    access_token: &SecretString,
) -> Option<UserinfoDocument> {
    let endpoint = request
        .userinfo_endpoint
        .as_deref()
        .filter(|endpoint| !endpoint.is_empty())?;
    let response = match fetch_userinfo(client, endpoint, request, access_token).await {
        Ok(response) => response,
        Err(failure) => {
            tracing::warn!(failure = %failure, "the SSO userinfo request failed; falling back to the id_token");
            return None;
        }
    };
    if response.status != 200 {
        tracing::warn!(
            status = response.status,
            "the SSO userinfo endpoint answered a non-success status; falling back to the id_token"
        );
        return None;
    }
    match decode_oidc_userinfo(&response.body) {
        Ok(UserinfoDocument::Empty) => {
            tracing::warn!(
                "the SSO userinfo endpoint answered empty claims; falling back to the id_token"
            );
            Some(UserinfoDocument::Empty)
        }
        Ok(document) => Some(document),
        Err(failure) => {
            tracing::warn!(failure = %failure, "the SSO userinfo response is not JSON; falling back to the id_token");
            None
        }
    }
}

async fn fetch_userinfo(
    client: &ProxiedClient,
    endpoint: &str,
    request: &TokenExchangeRequest,
    access_token: &SecretString,
) -> Result<UpstreamResponse, OidcTokenExchangeError> {
    let url = Url::parse(endpoint).map_err(OidcTokenExchangeError::UserinfoUrl)?;
    let mut headers = additional_headers(&request.additional_headers)?;
    let bearer = authorization_header(format!("Bearer {}", access_token.expose_secret()))?;
    headers.insert(AUTHORIZATION, bearer);
    let outbound = OutboundRequest {
        method: Method::GET,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Empty,
        deadline: USERINFO_TIMEOUT,
    };
    client
        .send_following(outbound)
        .await
        .map_err(OidcTokenExchangeError::UserinfoTransport)?
        .buffered(MAX_OIDC_RESPONSE_BYTES)
        .await
        .map_err(OidcTokenExchangeError::UserinfoTransport)
}
