use std::time::Duration;

use http::HeaderMap;
use secrecy::SecretString;

pub mod authorization;
pub mod discovery;
pub mod document;
pub mod error;
pub mod exchange;
pub mod google;
pub mod microsoft;

mod tests;

// httpx's AsyncClient default timeout, which fastapi-sso leaves on every provider request
pub const OAUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

// One login attempt's client configuration; the tokens stay on the exchange result, so overlapping attempts never share them.
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: SecretString,
    pub redirect_uri: Option<String>,
    pub scopes: Vec<String>,
    pub allow_insecure_http: bool,
    pub additional_headers: HeaderMap,
}
