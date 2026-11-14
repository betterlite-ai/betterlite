use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use http::header::ACCEPT;
use secrecy::ExposeSecret as _;
use tokio::runtime::Runtime;
use url::Url;

use self::error::FetchError;
use super::credentials::Credentials;
use super::session::Session;

pub mod error;

const SESSION_ENDPOINT: &str = "/auto_router/session";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
// A session answer is a few hundred bytes; the cap keeps a hostile gateway from filling memory
const BODY_LIMIT_BYTES: usize = 1_048_576;

// definitive: this credential's answer is settled for the cache TTL, so the next refresh need not ask again
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    pub session: Option<Session>,
    pub definitive: bool,
}

pub trait SessionFetch {
    fn fetch(&self, credentials: &Credentials, session_id: &str) -> Result<Fetched, FetchError>;
}

// Any 4xx is the credential's own answer (no row, no access, expired login); a 5xx or a 3xx is not
#[must_use]
pub fn classify(status: u16, body: &[u8]) -> Fetched {
    match status {
        200..=299 => {
            let session = Session::from_json(body);
            Fetched {
                definitive: session.is_some(),
                session,
            }
        }
        400..=499 => Fetched {
            session: None,
            definitive: true,
        },
        _ => Fetched::default(),
    }
}

pub fn session_url(base_url: &str, session_id: &str) -> Result<Url, FetchError> {
    Url::parse_with_params(
        &format!("{base_url}{SESSION_ENDPOINT}"),
        [("session_id", session_id)],
    )
    .map_err(FetchError::InvalidUrl)
}

pub struct HttpFetch<'client> {
    pub runtime: &'client Runtime,
    pub environment: &'client BTreeMap<String, String>,
}

impl SessionFetch for HttpFetch<'_> {
    fn fetch(&self, credentials: &Credentials, session_id: &str) -> Result<Fetched, FetchError> {
        let url = session_url(&credentials.base_url, session_id)?;
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), self.environment)
            .map_err(FetchError::Proxy)?;
        let client = ProxiedClient::outbound(&Arc::new(policy)).map_err(FetchError::Http)?;
        let request = client
            .client
            .get(url)
            .timeout(REQUEST_TIMEOUT)
            .bearer_auth(credentials.api_key.expose_secret())
            .header(ACCEPT, "application/json")
            .build()
            .map_err(|source| FetchError::Http(HttpError::Request(source)))?;
        // a transport failure, a timeout or an oversized body is not an answer, so the next refresh asks again
        Ok(
            match self
                .runtime
                .block_on(client.execute_buffered(request, BODY_LIMIT_BYTES))
            {
                Ok(response) => classify(response.status, &response.body),
                Err(_unreachable) => Fetched::default(),
            },
        )
    }
}

mod tests;
