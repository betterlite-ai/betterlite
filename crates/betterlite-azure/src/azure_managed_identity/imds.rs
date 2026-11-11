use std::io;

use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_client_secret::token::AccessToken;

use super::decode::{ImdsReply, ImdsTokenError, decode_imds_token};
use super::environment::ManagedIdentityEnvironment;
use super::error::ManagedIdentityError;
use super::retry::{RetryBudget, send};
use super::runtime::ImdsIo;

pub const IMDS_AUTHORITY: &str = "http://169.254.169.254";
pub const IMDS_TOKEN_PATH: &str = "/metadata/identity/oauth2/token";
pub const API_VERSION: &str = "2018-02-01";
pub const CLIENT_SKU: &str = "MSAL.Python";
pub const CLIENT_VERSION: &str = "1.37.0";
pub const CONNECTION_TIMEOUT_SECONDS: u64 = 2;
pub const PROBE_TIMEOUT_SECONDS: u64 = 1;
pub const TOKEN_BUDGET: RetryBudget = RetryBudget {
    total: 5,
    connect: 3,
};
const REFRESH_THRESHOLD_SECONDS: i64 = 7200;

// None means the transport's configured connection timeout, which is two seconds for the token path
#[derive(Debug)]
pub struct ImdsRequest {
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug)]
pub struct ImdsResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: SecretString,
}

pub trait ImdsTransport: Send + Sync {
    fn get(&self, request: &ImdsRequest) -> io::Result<ImdsResponse>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProbePolicy {
    pub enable: Option<bool>,
    pub within_credential_chain: bool,
}

impl ProbePolicy {
    #[must_use]
    pub const fn active(self) -> bool {
        match self.enable {
            Some(enable) => enable,
            None => self.within_credential_chain,
        }
    }
}

fn url(environment: &ManagedIdentityEnvironment) -> String {
    let authority = environment
        .pod_identity_authority_host
        .as_deref()
        .unwrap_or(IMDS_AUTHORITY);
    format!("{}{IMDS_TOKEN_PATH}", authority.trim_matches('/'))
}

fn query(resource: &str, client_id: Option<&str>) -> Vec<(String, String)> {
    let mut query = vec![
        ("api-version".to_owned(), API_VERSION.to_owned()),
        ("resource".to_owned(), resource.to_owned()),
    ];
    if let Some(identity) = client_id.filter(|value| !value.is_empty()) {
        query.push(("client_id".to_owned(), identity.to_owned()));
    }
    query
}

fn probe_request(
    environment: &ManagedIdentityEnvironment,
    resource: &str,
    client_id: Option<&str>,
) -> ImdsRequest {
    ImdsRequest {
        url: url(environment),
        query: query(resource, client_id),
        headers: Vec::new(),
        timeout_seconds: Some(PROBE_TIMEOUT_SECONDS),
    }
}

fn token_request(
    environment: &ManagedIdentityEnvironment,
    resource: &str,
    client_id: Option<&str>,
    request_id: &str,
) -> ImdsRequest {
    ImdsRequest {
        url: url(environment),
        query: query(resource, client_id),
        headers: vec![
            ("Metadata".to_owned(), "true".to_owned()),
            ("x-client-SKU".to_owned(), CLIENT_SKU.to_owned()),
            ("x-client-Ver".to_owned(), CLIENT_VERSION.to_owned()),
            ("x-ms-client-request-id".to_owned(), request_id.to_owned()),
        ],
        timeout_seconds: None,
    }
}

fn content_type(response: &ImdsResponse) -> Option<&str> {
    response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str())
}

const fn no_response() -> ManagedIdentityError {
    ManagedIdentityError::ImdsUnavailable {
        description: String::new(),
    }
}

fn check_probe(
    environment: &ManagedIdentityEnvironment,
    io: &ImdsIo<'_>,
    resource: &str,
    client_id: Option<&str>,
) -> Result<(), ManagedIdentityError> {
    let request = probe_request(environment, resource, client_id);
    let response = io
        .transport
        .get(&request)
        .map_err(|_source| no_response())?;
    if response.status != 403 {
        return Ok(());
    }
    let body = response.body.expose_secret();
    if body.contains("unreachable") {
        return Err(ManagedIdentityError::ImdsError {
            message: format!("Unexpected response \"{body}\""),
        });
    }
    Ok(())
}

fn reply_failure(response: &ImdsResponse, error: &ImdsTokenError) -> ManagedIdentityError {
    let ImdsTokenError::Json(_) = error else {
        return no_response();
    };
    let body = response.body.expose_secret();
    if body.is_empty() || content_type(response).is_some_and(|kind| kind.starts_with("text/")) {
        return ManagedIdentityError::AuthenticationUnavailable;
    }
    if response.status == 400 {
        return ManagedIdentityError::NoIdentityAssigned;
    }
    if response.status == 403 && body.contains("unreachable") {
        return ManagedIdentityError::ImdsError {
            message: format!("Unexpected response \"{body}\""),
        };
    }
    ManagedIdentityError::ClientAuthentication {
        message: "the IMDS token response is not JSON".to_owned(),
    }
}

fn reply(
    response: &ImdsResponse,
    started: i64,
    finished: i64,
) -> Result<AccessToken, ManagedIdentityError> {
    match decode_imds_token(response.body.expose_secret().as_bytes()) {
        Ok(ImdsReply::Token(token)) => {
            let expires_on = finished
                .checked_add(token.expires_in)
                .ok_or(ManagedIdentityError::ExpiryOverflow)?;
            let refresh_on = if token.expires_in >= REFRESH_THRESHOLD_SECONDS {
                Some(
                    started
                        .checked_add(token.expires_in.div_euclid(2))
                        .ok_or(ManagedIdentityError::ExpiryOverflow)?,
                )
            } else {
                None
            };
            Ok(AccessToken {
                token: token.access_token,
                expires_on,
                token_type: token.token_type,
                refresh_on,
            })
        }
        Ok(ImdsReply::Error { error, description }) => {
            Err(ManagedIdentityError::TokenRequest { error, description })
        }
        Ok(ImdsReply::Missing) => Err(no_response()),
        Err(error) => Err(reply_failure(response, &error)),
    }
}

pub fn acquire(
    environment: &ManagedIdentityEnvironment,
    io: &ImdsIo<'_>,
    resource: &str,
    client_id: Option<&str>,
    probe: ProbePolicy,
) -> Result<AccessToken, ManagedIdentityError> {
    // a configured pod authority host means the endpoint is known available, even when its value is empty
    let available = environment.pod_identity_authority_host.is_some();
    if probe.active() && !available {
        check_probe(environment, io, resource, client_id)?;
    }
    let request = token_request(environment, resource, client_id, &io.ids.next_id());
    let started = io.clock.now_unix_seconds();
    let response = send(io, &request, TOKEN_BUDGET)?;
    let finished = io.clock.now_unix_seconds();
    reply(&response, started, finished)
}
