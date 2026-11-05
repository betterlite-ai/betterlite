use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use config::{AuthMethod, BitBucketConfig};
use error::ClientError;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use http::{Method, StatusCode};
use secrecy::ExposeSecret as _;
use url::Url;

pub mod config;
pub mod content;
pub mod error;
pub mod path;
mod tests;

const DEFAULT_BRANCH: &str = "main";
const DEFAULT_BASE_URL: &str = "https://api.bitbucket.org/2.0";
// LiteLLM's HTTPHandler falls back to its 600 second completion timeout
const FILE_DEADLINE: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct BitBucketClient {
    pub workspace: String,
    pub repository: String,
    pub branch: String,
    pub base_url: String,
    pub headers: HeaderMap,
    pub transport: ProxiedClient,
}

impl BitBucketClient {
    pub fn new(config: BitBucketConfig, transport: ProxiedClient) -> Result<Self, ClientError> {
        if !config.unrecognized.is_empty() {
            let keys: Vec<&String> = config.unrecognized.keys().collect();
            tracing::warn!(?keys, "BitBucket config keys ignored");
        }
        let (Some(workspace), Some(repository), Some(access_token)) =
            (config.workspace, config.repository, config.access_token)
        else {
            return Err(ClientError::MissingRequired);
        };
        let token = access_token.expose_secret();
        if workspace.is_empty() || repository.is_empty() || token.is_empty() {
            return Err(ClientError::MissingRequired);
        }

        let basic_username = match config.auth_method {
            Some(AuthMethod::Basic) => config.username.filter(|username| !username.is_empty()),
            Some(AuthMethod::Token) | None => None,
        };
        let credentials = basic_username.map_or_else(
            || format!("Bearer {token}"),
            |username| format!("Basic {}", STANDARD.encode(format!("{username}:{token}"))),
        );
        let mut authorization =
            HeaderValue::from_str(&credentials).map_err(ClientError::InvalidAuthorization)?;
        authorization.set_sensitive(true);

        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(AUTHORIZATION, authorization);

        Ok(Self {
            workspace,
            repository,
            branch: config.branch.unwrap_or_else(|| DEFAULT_BRANCH.to_owned()),
            base_url: config
                .base_url
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned()),
            headers,
            transport,
        })
    }

    pub async fn get_file_content(&self, file_path: &str) -> Result<Option<String>, ClientError> {
        let safe_path = path::sanitize_file_path(file_path)?;
        let url = Url::parse(&format!(
            "{}/repositories/{}/{}/src/{}/{safe_path}",
            self.base_url, self.workspace, self.repository, self.branch
        ))
        .map_err(ClientError::InvalidUrl)?;
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: self.headers.clone(),
            body: RequestBody::Empty,
            deadline: FILE_DEADLINE,
        };
        let response = self
            .transport
            .send_buffered(request)
            .await
            .map_err(|source| ClientError::Transport {
                file_path: file_path.to_owned(),
                source,
            })?;

        if response.status == StatusCode::NOT_FOUND.as_u16() {
            return Ok(None);
        }
        self.check_status(response.status, file_path)?;
        let content_type = response
            .headers
            .get(CONTENT_TYPE)
            .map(HeaderValue::as_bytes);
        let text = content::decode_body(content_type, &response.body).map_err(|source| {
            ClientError::NotUtf8 {
                file_path: file_path.to_owned(),
                source,
            }
        })?;
        Ok(Some(text))
    }

    fn check_status(&self, status: u16, file_path: &str) -> Result<(), ClientError> {
        match StatusCode::from_u16(status) {
            Ok(code) if code.is_success() => Ok(()),
            Ok(StatusCode::FORBIDDEN) => Err(ClientError::AccessDenied {
                file_path: file_path.to_owned(),
                workspace: self.workspace.clone(),
                repository: self.repository.clone(),
            }),
            Ok(StatusCode::UNAUTHORIZED) => Err(ClientError::AuthenticationFailed),
            Ok(_) | Err(_) => Err(ClientError::Status {
                file_path: file_path.to_owned(),
                status,
            }),
        }
    }
}
