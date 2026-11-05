use std::error::Error;
use std::fmt;
use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::header::AUTHORIZATION;
use http::{HeaderMap, HeaderValue, Method};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use tokio::sync::OnceCell;
use url::{Url, form_urlencoded};

use crate::proxy_auth::clock::Clock;
use crate::proxy_auth::error::{AzureCredentialError, OAuth2Error, ProxyAuthError};

pub mod clock;
pub mod error;

// LiteLLM treats a token whose expires_on is less than 60 seconds away as expired, in the OAuth2 cache and in the handler alike
const REFRESH_BUFFER_SECONDS: i64 = 60;

#[derive(Clone)]
pub struct AccessToken {
    pub token: secrecy::SecretString,
    pub expires_on: i64,
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("token", &"[REDACTED]")
            .field("expires_on", &self.expires_on)
            .finish()
    }
}

pub trait TokenCredential: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn get_token(
        &self,
        scope: &str,
    ) -> impl Future<Output = Result<AccessToken, Self::Error>> + Send;
}

async fn cached_or_fetched<E, F, Fut>(
    cache: &Mutex<Option<AccessToken>>,
    now: i64,
    fetch: F,
) -> Result<AccessToken, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<AccessToken, E>>,
{
    let fresh = cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .filter(|cached| cached.expires_on > now.saturating_add(REFRESH_BUFFER_SECONDS));
    if let Some(cached) = fresh {
        return Ok(cached);
    }
    let token = fetch().await?;
    *cache.lock().unwrap_or_else(PoisonError::into_inner) = Some(token.clone());
    Ok(token)
}

// default_chain is DefaultAzureCredential, built on the first get_token call; None stands for a missing azure-identity package
pub struct AzureADCredential<C: TokenCredential> {
    pub credential: OnceCell<C>,
    pub default_chain: Option<fn() -> C>,
}

impl<C: TokenCredential> TokenCredential for AzureADCredential<C> {
    type Error = AzureCredentialError<C::Error>;

    async fn get_token(&self, scope: &str) -> Result<AccessToken, Self::Error> {
        let resolved = self
            .credential
            .get_or_try_init(|| async {
                self.default_chain
                    .map(|build| build())
                    .ok_or(AzureCredentialError::ImportError)
            })
            .await?;
        resolved
            .get_token(scope)
            .await
            .map_err(AzureCredentialError::Credential)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: secrecy::SecretString,
    expires_in: Option<i64>,
}

// The cache is not keyed by scope: a token fetched for one scope answers every later scope, as in LiteLLM
pub struct GenericOAuth2Credential<K: Clock> {
    pub client_id: String,
    pub client_secret: secrecy::SecretString,
    pub token_url: Url,
    pub client: ProxiedClient,
    pub clock: K,
    pub cached_token: Mutex<Option<AccessToken>>,
}

impl<K: Clock> fmt::Debug for GenericOAuth2Credential<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenericOAuth2Credential")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .field("token_url", &self.token_url)
            .finish_non_exhaustive()
    }
}

impl<K: Clock> GenericOAuth2Credential<K> {
    fn token_request(&self, scope: &str) -> OutboundRequest {
        let form = form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "client_credentials")
            .append_pair("client_id", &self.client_id)
            .append_pair("client_secret", self.client_secret.expose_secret())
            .append_pair("scope", scope)
            .finish();
        OutboundRequest {
            method: Method::POST,
            url: self.token_url.clone(),
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
                data: form.into_bytes(),
            },
            deadline: self.client.limits.read_timeout,
        }
    }

    async fn exchange(&self, scope: &str) -> Result<AccessToken, OAuth2Error> {
        let response = self
            .client
            .send_buffered(self.token_request(scope))
            .await
            .map_err(OAuth2Error::Transport)?;
        if !(200..300).contains(&response.status) {
            return Err(OAuth2Error::Status {
                status: response.status,
            });
        }
        let parsed: TokenResponse =
            serde_json::from_slice(&response.body).map_err(OAuth2Error::Body)?;
        // a token response without expires_in lasts an hour
        let lifetime = parsed.expires_in.unwrap_or(3600);
        let expires_on = self
            .clock
            .unix_seconds()
            .checked_add(lifetime)
            .ok_or(OAuth2Error::ExpiryOverflow)?;
        Ok(AccessToken {
            token: parsed.access_token,
            expires_on,
        })
    }
}

impl<K: Clock> TokenCredential for GenericOAuth2Credential<K> {
    type Error = OAuth2Error;

    async fn get_token(&self, scope: &str) -> Result<AccessToken, OAuth2Error> {
        let now = self.clock.unix_seconds();
        cached_or_fetched(&self.cached_token, now, || self.exchange(scope)).await
    }
}

pub struct ProxyAuthHandler<C: TokenCredential, K: Clock> {
    pub credential: C,
    pub scope: String,
    pub clock: K,
    pub cached_token: Mutex<Option<AccessToken>>,
}

impl<C: TokenCredential, K: Clock> fmt::Debug for ProxyAuthHandler<C, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyAuthHandler")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl<C: TokenCredential, K: Clock> ProxyAuthHandler<C, K> {
    pub async fn get_token(&self) -> Result<AccessToken, C::Error> {
        let now = self.clock.unix_seconds();
        cached_or_fetched(&self.cached_token, now, || {
            self.credential.get_token(&self.scope)
        })
        .await
    }

    pub async fn get_auth_headers(&self) -> Result<HeaderMap, ProxyAuthError<C::Error>> {
        let token = self.get_token().await.map_err(ProxyAuthError::Credential)?;
        let mut bearer = HeaderValue::try_from(format!("Bearer {}", token.token.expose_secret()))
            .map_err(ProxyAuthError::InvalidHeader)?;
        bearer.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, bearer);
        Ok(headers)
    }
}

mod tests;
