use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
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
const ACCEPT_JSON: &str = "application/json";
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
pub(crate) fn classify(status: u16, body: &[u8]) -> Fetched {
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
// A transport failure, a timeout or an oversized body is not an answer, so the next refresh asks again
pub(crate) fn fetched_answer(answer: Result<UpstreamResponse, HttpError>) -> Fetched {
    match answer {
        Ok(response) => classify(response.status, &response.body),
        Err(_) => Fetched::default(),
    }
}
pub(crate) fn session_url(base_url: &str, session_id: &str) -> Result<Url, FetchError> {
    Url::parse_with_params(
        &format!("{base_url}{SESSION_ENDPOINT}"),
        [("session_id", session_id)],
    )
    .map_err(FetchError::InvalidUrl)
}

// urllib, which LiteLLM's script uses, takes the proxy from the *_proxy variables and nothing else
fn proxy_environment(environment: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut selected = environment.clone();
    selected.retain(|name, _value| name.to_ascii_lowercase().ends_with("_proxy"));
    selected
}

// One expansion builds the request in production and in the tests, so its timeout, key and Accept are checked
macro_rules! session_request {
    ($client:expr, $url:expr, $key:expr) => {
        $client
            .client
            .get($url)
            .timeout(REQUEST_TIMEOUT)
            .bearer_auth($key)
            .header(ACCEPT, ACCEPT_JSON)
            .build()
            .map_err(|source| FetchError::Http(HttpError::Request(source)))
    };
}
pub struct HttpFetch<'client> {
    pub runtime: &'client Runtime,
    pub environment: &'client BTreeMap<String, String>,
}

impl SessionFetch for HttpFetch<'_> {
    fn fetch(&self, credentials: &Credentials, session_id: &str) -> Result<Fetched, FetchError> {
        let url = session_url(&credentials.base_url, session_id)?;
        let policy =
            ProxyPolicy::resolve(&ProxyTrust::default(), &proxy_environment(self.environment))
                .map_err(FetchError::Proxy)?;
        let client = ProxiedClient::outbound(&Arc::new(policy)).map_err(FetchError::Http)?;
        let request = session_request!(client, url, credentials.api_key.expose_secret())?;
        Ok(fetched_answer(self.runtime.block_on(
            client.execute_buffered(request, BODY_LIMIT_BYTES),
        )))
    }
}

mod tests;
