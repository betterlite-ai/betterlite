use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::transport_error::TransportError;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

use super::cache::{CachedLookup, CachedPrompt, Clock, PromptCache, PromptKey};
use super::errors::{AuthCheckFailure, LangfuseRestError, auth_check_failure, decode_response};
use super::records::{Projects, Prompt};
use super::transport::{LangfuseHttp, RequestFailure};

const PROJECTS_PATH: &str = "api/public/projects";
const PROMPTS_PATH: &str = "api/public/v2/prompts";
// Python's urllib.parse.quote(name, safe="") leaves only the RFC 3986 unreserved characters alone
const PROMPT_NAME_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub struct LangfuseClientConfig {
    pub public_key: Option<String>,
    pub secret_key: Option<SecretString>,
    pub base_url: String,
    pub timeout: Duration,
    pub prompt_cache_ttl: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum LangfuseClientError {
    #[error("the Langfuse HTTP client could not be built")]
    Client {
        #[source]
        source: HttpError,
    },
    #[error("the proxy settings for the Langfuse HTTP client could not be resolved")]
    Proxy {
        #[source]
        source: EnvError,
    },
}

pub struct LangfuseApiClient {
    tasks: TaskTracker,
    state: Arc<ClientState>,
}

struct ClientState {
    http: LangfuseHttp,
    prompt_cache_ttl: Duration,
    clock: Arc<dyn Clock>,
    cache: Mutex<PromptCache>,
}

pub fn build_langfuse_client(
    config: LangfuseClientConfig,
    httpx_client: Option<ProxiedClient>,
    clock: Arc<dyn Clock>,
    tasks: TaskTracker,
) -> Result<LangfuseApiClient, LangfuseClientError> {
    let client = match httpx_client {
        Some(client) => client,
        None => default_client()?,
    };
    let http = LangfuseHttp {
        client,
        base_url: config.base_url,
        public_key: config.public_key,
        secret_key: config.secret_key,
        timeout: config.timeout,
    };
    Ok(LangfuseApiClient {
        tasks,
        state: Arc::new(ClientState {
            http,
            prompt_cache_ttl: config.prompt_cache_ttl,
            clock,
            cache: Mutex::new(PromptCache::default()),
        }),
    })
}

fn default_client() -> Result<ProxiedClient, LangfuseClientError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .map_err(|source| LangfuseClientError::Proxy { source })?;
    ProxiedClient::outbound(&Arc::new(policy))
        .map_err(|source| LangfuseClientError::Client { source })
}

impl LangfuseApiClient {
    pub async fn auth_check(&self) -> Option<AuthCheckFailure> {
        let response = match self.state.http.get(PROJECTS_PATH, Vec::new()).await {
            Ok(response) => response,
            Err(failure) => return Some(auth_check_failure(failure.to_string())),
        };
        let projects: Projects = match decode_response(&response) {
            Ok(projects) => projects,
            Err(error) => return Some(auth_check_failure(error.to_string())),
        };
        if projects.data.is_empty() {
            return Some(auth_check_failure(
                "no project found for the keys provided".to_owned(),
            ));
        }
        None
    }

    pub async fn project_id(&self) -> Result<Option<String>, LangfuseRestError> {
        let response = self.state.http.get(PROJECTS_PATH, Vec::new()).await?;
        let projects: Projects = decode_response(&response)?;
        Ok(projects.data.into_iter().next().map(|project| project.id))
    }

    pub async fn get_prompt(
        &self,
        name: &str,
        label: Option<&str>,
        version: Option<i64>,
    ) -> Result<Prompt, LangfuseRestError> {
        let key = PromptKey {
            name: name.to_owned(),
            version,
            label: label.map(str::to_owned),
        };
        let now = self.state.clock.now();
        let cached = {
            let cache = self
                .state
                .cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            cache.lookup(&key, self.state.prompt_cache_ttl, now)
        };
        match cached {
            Some(CachedLookup::Fresh(prompt)) => Ok(prompt),
            Some(CachedLookup::Stale(prompt)) => {
                self.refresh_in_background(key);
                Ok(prompt)
            }
            None => self.state.fetch(&key).await,
        }
    }

    fn refresh_in_background(&self, key: PromptKey) {
        let mut cache = self
            .state
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let claimed = cache.refreshing.insert(key.clone());
        drop(cache);
        if !claimed {
            return;
        }
        let state = Arc::clone(&self.state);
        let _refresh = self.tasks.spawn(async move {
            state.refresh(key).await;
        });
    }
}

impl ClientState {
    async fn fetch(&self, key: &PromptKey) -> Result<Prompt, LangfuseRestError> {
        let prompt = self.request_prompt(key).await?;
        self.cache_prompt(key, prompt.clone());
        Ok(prompt)
    }

    async fn request_prompt(&self, key: &PromptKey) -> Result<Prompt, LangfuseRestError> {
        match self.fetch_once(key).await {
            Ok(prompt) => return Ok(prompt),
            Err(error) if is_retryable(&error) => {
                tracing::debug!(
                    "Langfuse prompt '{}' fetch failed ({error}), retrying once",
                    key.name
                );
            }
            Err(error) => return Err(error),
        }
        self.fetch_once(key).await
    }

    async fn fetch_once(&self, key: &PromptKey) -> Result<Prompt, LangfuseRestError> {
        let path = format!(
            "{PROMPTS_PATH}/{}",
            utf8_percent_encode(&key.name, PROMPT_NAME_SET)
        );
        let response = self.http.get(&path, prompt_query(key)).await?;
        decode_response(&response)
    }

    fn cache_prompt(&self, key: &PromptKey, prompt: Prompt) {
        let now = self.clock.now();
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.entries.insert(
            key.clone(),
            CachedPrompt {
                prompt,
                fetched_at: now,
            },
        );
    }

    async fn refresh(&self, key: PromptKey) {
        if let Err(error) = self.fetch(&key).await {
            tracing::warn!(
                "Langfuse prompt '{}' refresh failed, serving the cached version: {error}",
                key.name
            );
        }
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.refreshing.remove(&key);
    }
}

fn prompt_query(key: &PromptKey) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(version) = key.version {
        query.push(("version".to_owned(), version.to_string()));
    }
    if let Some(label) = &key.label {
        query.push(("label".to_owned(), label.clone()));
    }
    query
}

const fn is_retryable(error: &LangfuseRestError) -> bool {
    match error {
        LangfuseRestError::Api(api) => api.status_code >= 500,
        LangfuseRestError::Request(failure) => request_is_retryable(failure),
        LangfuseRestError::Decode { .. } | LangfuseRestError::Body { .. } => false,
    }
}

const fn request_is_retryable(failure: &RequestFailure) -> bool {
    match failure {
        RequestFailure::Url { .. } | RequestFailure::Decode(_) => false,
        RequestFailure::Transport(error) => transport_is_retryable(error),
    }
}

// httpx counts these as TransportError; TooManyRedirects, InvalidURL and DecodingError are RequestErrors
const fn transport_is_retryable(error: &TransportError) -> bool {
    match error {
        TransportError::Connect(_)
        | TransportError::ConnectTimeout(_)
        | TransportError::Deadline(_)
        | TransportError::Request(_)
        | TransportError::InvalidHeader { .. }
        | TransportError::Body(_)
        | TransportError::RedirectScheme { .. } => true,
        TransportError::Rejected(_)
        | TransportError::Build(_)
        | TransportError::HeadersTooLarge { .. }
        | TransportError::BodyTooLarge { .. }
        | TransportError::MultipartContentType { .. }
        | TransportError::MultipartBoundaryHeader { .. }
        | TransportError::MultipartBoundary { .. }
        | TransportError::Mock(_)
        | TransportError::Closed
        | TransportError::DrainTimeout { .. }
        | TransportError::TooManyRedirects { .. }
        | TransportError::RedirectTarget { .. }
        | TransportError::RedirectUrl { .. }
        | TransportError::RedirectUserinfo
        | TransportError::InsecureRedirect
        | TransportError::RedirectBody => false,
    }
}
