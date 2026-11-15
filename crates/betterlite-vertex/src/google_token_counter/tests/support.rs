#![cfg(test)]

use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use http::header::HeaderMap;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

use crate::credential_sources::sdk::CredentialClock;
use crate::google_count_tokens::transport::CountTransport;
use crate::google_token_counter::counter::{CountRuntime, CountTokensCall, TokenCounter as _};
use crate::google_token_counter::error::CountTokensError;
use crate::google_token_counter::reply::TokenCountResponse;
use crate::partner_count::execute::PartnerCountTransport;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::probe::AccessToken;
use crate::token_cache::record::CredentialSnapshot;
use crate::token_cache::state::TokenState;
use crate::token_refresh::CredentialRefresh;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};

use super::vertex_counter;

pub(super) const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(super) struct Sent {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

impl Sent {
    pub(super) fn body_json(&self) -> Json {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub(super) fn header(&self, name: &str) -> Option<String> {
        let value = self.headers.get(name)?;
        Some(value.to_str().ok()?.to_owned())
    }
}

pub(super) struct Script {
    reply: Vec<u8>,
    sent: Mutex<Vec<Sent>>,
}

impl Script {
    pub(super) fn replying(body: &[u8]) -> Self {
        Self {
            reply: body.to_vec(),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, request: OutboundRequest) -> UpstreamResponse {
        let body = match request.body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        self.sent.lock().unwrap().push(Sent {
            url: request.url.to_string(),
            headers: request.headers,
            body,
        });
        UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: self.reply.clone(),
        }
    }

    pub(super) fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }
}

impl CountTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(self.record(request)))
    }
}

impl PartnerCountTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(self.record(request)))
    }
}

pub(super) struct NoMedia;

impl RemoteMediaFetcher for NoMedia {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: urls.join(","),
        }))
    }
}

pub(super) static NO_MEDIA: NoMedia = NoMedia;

#[derive(Default)]
pub(super) struct RecordingMint {
    seen: Mutex<Vec<(Option<String>, Option<String>)>>,
}

impl RecordingMint {
    pub(super) fn seen(&self) -> Vec<(Option<String>, Option<String>)> {
        self.seen.lock().unwrap().clone()
    }
}

impl MintAccessToken for RecordingMint {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        self.seen.lock().unwrap().push((
            credentials
                .and_then(ConfiguredCredentials::as_text)
                .map(str::to_owned),
            project.map(str::to_owned),
        ));
        Box::pin(ready(Ok(AccessToken {
            token: SecretString::from("minted"),
            project: project.unwrap_or("minted-project").to_owned(),
        })))
    }
}

struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

#[derive(Default)]
pub(super) struct StaticExecutor {
    seen: Mutex<Vec<(Option<String>, Option<String>)>>,
}

impl StaticExecutor {
    pub(super) fn seen(&self) -> Vec<(Option<String>, Option<String>)> {
        self.seen.lock().unwrap().clone()
    }
}

impl CredentialExecutor for StaticExecutor {
    fn load<'borrow>(
        &'borrow self,
        credentials: Option<&'borrow ConfiguredCredentials>,
        project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.seen.lock().unwrap().push((
            credentials
                .and_then(ConfiguredCredentials::as_text)
                .map(str::to_owned),
            project.map(str::to_owned),
        ));
        let resolved = project.unwrap_or("proj").to_owned();
        let snapshot = CredentialSnapshot {
            token: Some(SecretString::from("vertex-token")),
            state: Some(TokenState::Fresh),
            expires_at: None,
            expired: None,
            valid: None,
            quota_project_id: None,
            project_id: Some(resolved.clone()),
        };
        Box::pin(ready(Ok(LoadedCredential {
            credential: Some(Arc::new(snapshot)),
            project: Some(resolved),
        })))
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(ready(Err(CredentialCacheError::MissingToken)))
    }
}

pub(super) fn refresh() -> CredentialRefresh {
    refresh_from(Arc::new(StaticExecutor::default()))
}

pub(super) fn refresh_from(executor: Arc<StaticExecutor>) -> CredentialRefresh {
    CredentialRefresh::new(
        Arc::new(CredentialCache::new(Arc::new(FixedClock), executor)),
        TaskTracker::new(),
    )
}

pub(super) fn runtime<'run>(
    script: &'run Script,
    mint: &'run RecordingMint,
    refresh: &'run CredentialRefresh,
    catalog: &'run ModelCatalog,
) -> CountRuntime<'run, Script, RecordingMint, NoMedia> {
    CountRuntime {
        transport: script,
        refresh,
        catalog,
        mint,
        fetcher: &NO_MEDIA,
    }
}

pub(super) async fn vertex_count(
    script: &Script,
    mint: &RecordingMint,
    refresh: &CredentialRefresh,
    catalog: &ModelCatalog,
    request: &CountTokensCall<'_>,
) -> Result<Option<TokenCountResponse>, CountTokensError> {
    let counter = vertex_counter();
    let live = runtime(script, mint, refresh, catalog);
    counter.count_tokens(&live, request, DEADLINE).await
}
