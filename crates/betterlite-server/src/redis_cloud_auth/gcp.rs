use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::redis_credentials::gcp::GcpTokenFetcher;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::runtime::Handle;
use url::{ParseError, Url};

use betterlite_vertex::credential_sources::error::CredentialSourceError;

pub const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
pub const IAM_CREDENTIALS_BASE: &str = "https://iamcredentials.googleapis.com";
const IAM_TIMEOUT: Duration = Duration::from_secs(30);

pub trait AdcTokenSource: Send + Sync {
    fn access_token(&self, scope: &str) -> Result<SecretString, AdcError>;
}

#[derive(Debug, Error)]
pub enum AdcError {
    #[error("the Google application default credentials could not supply a token: {source}")]
    Adc {
        #[source]
        source: CredentialSourceError,
    },
}

#[derive(Debug, Error)]
pub enum IamTokenError {
    #[error("the Google application default credentials could not supply a token: {source}")]
    Adc {
        #[source]
        source: AdcError,
    },
    #[error("the IAM generateAccessToken request could not be built: {source}")]
    Url {
        #[source]
        source: ParseError,
    },
    #[error(
        "the application default credentials token cannot be carried in a bearer header: {source}"
    )]
    Bearer {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the IAM generateAccessToken request body could not be encoded: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("the IAM generateAccessToken request failed: {source}")]
    Request {
        #[source]
        source: TransportError,
    },
    #[error("the IAM generateAccessToken request returned HTTP status {status}")]
    Status { status: u16 },
    #[error("the IAM generateAccessToken reply is not the JSON object the API returns")]
    Reply {
        #[source]
        source: serde_json::Error,
    },
    #[error("the IAM generateAccessToken reply has no accessToken string")]
    MissingToken,
}

pub struct IamTokenFetcher {
    pub runtime: Handle,
    pub http: ProxiedClient,
    pub base: String,
    pub adc: Arc<dyn AdcTokenSource>,
}

impl GcpTokenFetcher for IamTokenFetcher {
    type Error = IamTokenError;

    fn fetch(&self, service_account: &str) -> Result<SecretString, Self::Error> {
        let adc = self
            .adc
            .access_token(CLOUD_PLATFORM_SCOPE)
            .map_err(|source| IamTokenError::Adc { source })?;
        let url = Url::parse(&format!(
            "{}/v1/{service_account}:generateAccessToken",
            self.base.trim_end_matches('/')
        ))
        .map_err(|source| IamTokenError::Url { source })?;
        let mut headers = HeaderMap::new();
        let bearer = HeaderValue::from_str(&format!("Bearer {}", adc.expose_secret()))
            .map_err(|source| IamTokenError::Bearer { source })?;
        headers.insert(AUTHORIZATION, bearer);
        let body = serde_json::to_vec(&GenerateAccessTokenRequest {
            scope: [CLOUD_PLATFORM_SCOPE],
        })
        .map_err(|source| IamTokenError::Body { source })?;
        let request = OutboundRequest {
            method: http::Method::POST,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Json(body),
            deadline: IAM_TIMEOUT,
        };
        let response = self
            .runtime
            .block_on(self.http.send_buffered(request))
            .map_err(|source| IamTokenError::Request { source })?;
        if !(200..300).contains(&response.status) {
            return Err(IamTokenError::Status {
                status: response.status,
            });
        }
        decode_iam_token(&response.body)
    }
}

#[derive(serde::Serialize)]
struct GenerateAccessTokenRequest<'scope> {
    scope: [&'scope str; 1],
}

#[derive(serde::Deserialize)]
struct IamTokenReply {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
}

pub fn decode_iam_token(body: &[u8]) -> Result<SecretString, IamTokenError> {
    let reply: IamTokenReply =
        serde_json::from_slice(body).map_err(|source| IamTokenError::Reply { source })?;
    reply
        .access_token
        .map(SecretString::from)
        .ok_or(IamTokenError::MissingToken)
}
