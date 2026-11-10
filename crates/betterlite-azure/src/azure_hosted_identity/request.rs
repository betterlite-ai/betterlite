use std::collections::BTreeMap;
use std::io;

use betterlite_redact::secret::Redactor;
use secrecy::SecretString;

use crate::azure_client_secret::token::AccessToken;
use crate::azure_client_secret::transport::{Clock, HttpMethod};

use super::error::HostedIdentityError;

pub const APP_SERVICE_API_VERSION: &str = "2019-08-01";
pub const AZURE_ML_API_VERSION: &str = "2017-09-01";
pub const SERVICE_FABRIC_API_VERSION: &str = "2019-07-01-preview";
pub const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

#[derive(Debug)]
pub struct HostedRequest {
    pub method: HttpMethod,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(&'static str, SecretString)>,
    pub form: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct HostedResponse {
    pub content_type: Option<String>,
    pub body: SecretString,
}

pub trait HostedTransport {
    fn send(&self, request: &HostedRequest) -> io::Result<HostedResponse>;
}

pub type TokenResult = Result<AccessToken, HostedIdentityError>;

#[derive(Clone, Copy)]
pub struct HostedIo<'io> {
    pub transport: &'io dyn HostedTransport,
    pub clock: &'io dyn Clock,
    pub redactor: &'io Redactor,
}

impl HostedIo<'_> {
    pub fn exchange(
        &self,
        request: &HostedRequest,
    ) -> Result<(HostedResponse, i64), HostedIdentityError> {
        let started = self.clock.now_unix_seconds();
        let response = self
            .transport
            .send(request)
            .map_err(HostedIdentityError::Transport)?;
        Ok((response, started))
    }
}

#[must_use]
pub fn token_request(
    endpoint: &str,
    api_version: &'static str,
    resource: &str,
    identity_param: Option<(&'static str, &str)>,
    headers: Vec<(&'static str, SecretString)>,
) -> HostedRequest {
    let mut query = vec![
        ("api-version".to_owned(), api_version.to_owned()),
        ("resource".to_owned(), resource.to_owned()),
    ];
    if let Some((name, value)) = identity_param {
        query.push((name.to_owned(), value.to_owned()));
    }
    HostedRequest {
        method: HttpMethod::Get,
        url: endpoint.to_owned(),
        query,
        headers,
        form: Vec::new(),
    }
}

#[must_use]
pub fn cloud_shell_request(
    endpoint: &str,
    resource: &str,
    identity_config: &BTreeMap<String, String>,
    client_id: Option<&str>,
) -> HostedRequest {
    let mut form = vec![("resource".to_owned(), resource.to_owned())];
    form.extend(identity_config.clone());
    if let Some(identifier) = client_id {
        form.push(("client_id".to_owned(), identifier.to_owned()));
    }
    HostedRequest {
        method: HttpMethod::Post,
        url: endpoint.to_owned(),
        query: Vec::new(),
        headers: vec![
            ("Metadata", SecretString::from("true")),
            ("Content-Type", SecretString::from(FORM_CONTENT_TYPE)),
        ],
        form,
    }
}
