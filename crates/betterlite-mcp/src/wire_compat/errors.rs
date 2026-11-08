use std::collections::BTreeMap;
use std::fmt::Debug;

use thiserror::Error;

use crate::credential_contracts::errors::CredError;

pub const URL_CREDENTIALS_STATUS: u16 = 500;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpException {
    pub status_code: u16,
    pub detail: String,
    pub headers: BTreeMap<String, String>,
}

#[must_use]
pub fn url_credentials_exception() -> HttpException {
    HttpException {
        status_code: URL_CREDENTIALS_STATUS,
        detail: CredError::UrlCredentialsNotAllowed.to_string(),
        headers: BTreeMap::new(),
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Upstream MCP server '{server_name}' returned {status_code}")]
pub struct UpstreamAuthError {
    pub status_code: u16,
    pub www_authenticate: Option<String>,
    pub server_name: String,
}

impl UpstreamAuthError {
    #[must_use]
    pub fn to_http_exception(
        &self,
        base_url: Option<&str>,
        request_path: Option<&str>,
    ) -> HttpException {
        let mut challenge = self.www_authenticate.clone();
        // a relative resource_metadata is rejected by strict clients, so an empty origin emits no challenge
        if challenge.is_none()
            && self.status_code == 401
            && let Some(origin) = base_url.filter(|base| !base.is_empty())
        {
            let prefix = origin.trim_end_matches('/');
            let legacy_path = format!("/{}/mcp", self.server_name);
            let resource_metadata_url =
                if request_path.is_some_and(|path| path.starts_with(&legacy_path)) {
                    format!(
                        "{prefix}/.well-known/oauth-protected-resource/{}/mcp",
                        self.server_name
                    )
                } else {
                    format!(
                        "{prefix}/.well-known/oauth-protected-resource/mcp/{}",
                        self.server_name
                    )
                };
            challenge = Some(format!(
                "Bearer resource_metadata=\"{resource_metadata_url}\""
            ));
        }
        let detail = if self.status_code == 403 {
            "Forbidden"
        } else {
            "Unauthorized"
        };
        HttpException {
            status_code: self.status_code,
            detail: detail.to_owned(),
            headers: challenge
                .map(|value| BTreeMap::from([("www-authenticate".to_owned(), value)]))
                .unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("upstream returned HTTP {status_code}")]
pub struct OpenApiUpstreamError {
    pub status_code: u16,
    pub server_name: String,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("")]
pub struct ToolResultError;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Listing tools from MCP server '{server_name}' failed")]
pub struct ServerListError<Fault: Debug> {
    pub fault: Fault,
    pub server_name: String,
}

mod tests;
