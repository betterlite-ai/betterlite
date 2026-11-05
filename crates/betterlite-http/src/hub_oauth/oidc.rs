use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::huggingface_hub::Settings as HubSettings;
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Method, Url};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::hub_oauth::clock::HubClock;
use crate::hub_oauth::error::OidcError;
use crate::proxy::ProxiedClient;
use crate::proxy::outbound::{OutboundRequest, RequestBody};

const REFRESH_MARGIN: u64 = 300;
const DEFAULT_EXPIRES_IN: i64 = 3600;
const TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ID_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";

#[derive(Deserialize)]
struct GithubToken {
    value: String,
}

#[derive(Serialize)]
struct ExchangeBody<'body> {
    grant_type: &'body str,
    subject_token_type: &'body str,
    subject_token: &'body str,
    resource: &'body str,
}

#[derive(Deserialize)]
struct Exchanged {
    access_token: String,
    expires_in: Option<i64>,
}

struct CachedToken {
    token: SecretString,
    expires_at: u64,
}

pub struct OidcExchange {
    hub: Arc<HubSettings>,
    settings: OidcSettings,
    client: Arc<ProxiedClient>,
    clock: Arc<dyn HubClock>,
    cache: Mutex<Option<CachedToken>>,
}

impl OidcExchange {
    #[must_use]
    pub fn new(
        hub: Arc<HubSettings>,
        settings: OidcSettings,
        client: Arc<ProxiedClient>,
        clock: Arc<dyn HubClock>,
    ) -> Self {
        Self {
            hub,
            settings,
            client,
            clock,
            cache: Mutex::new(None),
        }
    }

    // huggingface_hub's _get_token_from_oidc: None when HF_OIDC_RESOURCE is unset, every failure otherwise is an error
    pub async fn token(&self) -> Result<Option<SecretString>, OidcError> {
        let Some(resource) = &self.settings.resource else {
            return Ok(None);
        };
        let mut cache = self.cache.lock().await;
        let now = self.clock.monotonic_seconds();
        if let Some(cached) = cache.as_ref()
            && now < cached.expires_at
        {
            return Ok(Some(cached.token.clone()));
        }
        let subject_token = match &self.settings.id_token {
            Some(token) => token.clone(),
            None if self.settings.github_actions => self.github_token().await?,
            None => return Err(OidcError::NoIdToken),
        };
        let exchanged: Exchanged = self.exchange(resource, &subject_token).await?;
        // a supplied HF_OIDC_ID_TOKEN cannot be minted again, so it is cached for the whole lifetime
        let margin = if self.settings.id_token.is_some() {
            0
        } else {
            REFRESH_MARGIN
        };
        let lifetime = u64::try_from(exchanged.expires_in.unwrap_or(DEFAULT_EXPIRES_IN))
            .unwrap_or(0)
            .saturating_sub(margin);
        let token = SecretString::from(exchanged.access_token);
        *cache = Some(CachedToken {
            token: token.clone(),
            expires_at: now.saturating_add(lifetime),
        });
        drop(cache);
        Ok(Some(token))
    }

    async fn github_token(&self) -> Result<SecretString, OidcError> {
        let (Some(request_url), Some(request_token)) =
            (&self.settings.request_url, &self.settings.request_token)
        else {
            return Err(OidcError::GithubRequest);
        };
        let mut bearer = HeaderValue::try_from(format!("Bearer {}", request_token.expose_secret()))
            .map_err(OidcError::Header)?;
        bearer.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, bearer);
        let request = OutboundRequest {
            method: Method::GET,
            url: Url::parse(request_url).map_err(|source| OidcError::Url {
                url: request_url.clone(),
                source,
            })?,
            query: vec![("audience".to_owned(), self.hub.endpoint.clone())],
            headers,
            body: RequestBody::Empty,
            deadline: Duration::from_secs(self.hub.download_timeout),
        };
        let minted: GithubToken = self.fetch(request).await?;
        Ok(SecretString::from(minted.value))
    }

    async fn exchange(
        &self,
        resource: &str,
        subject_token: &SecretString,
    ) -> Result<Exchanged, OidcError> {
        let url = format!("{}/oauth/token", self.hub.endpoint);
        let exchange = ExchangeBody {
            grant_type: TOKEN_EXCHANGE_GRANT_TYPE,
            subject_token_type: ID_TOKEN_TYPE,
            subject_token: subject_token.expose_secret(),
            resource,
        };
        let body = serde_json::to_vec(&exchange).map_err(|source| OidcError::Body {
            url: url.clone(),
            source,
        })?;
        let request = OutboundRequest {
            method: Method::POST,
            url: Url::parse(&url).map_err(|source| OidcError::Url { url, source })?,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Json(body),
            deadline: Duration::from_secs(self.hub.download_timeout),
        };
        self.fetch(request).await
    }

    async fn fetch<T>(&self, request: OutboundRequest) -> Result<T, OidcError>
    where
        T: DeserializeOwned,
    {
        let url = request.url.to_string();
        let response =
            self.client
                .send_buffered(request)
                .await
                .map_err(|source| OidcError::Transport {
                    url: url.clone(),
                    source,
                })?;
        if !(200..300).contains(&response.status) {
            return Err(OidcError::Status {
                url,
                status: response.status,
            });
        }
        serde_json::from_slice(&response.body).map_err(|source| OidcError::Body { url, source })
    }
}
