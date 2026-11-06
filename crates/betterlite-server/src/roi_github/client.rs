use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::http::header::{ACCEPT, AUTHORIZATION};
use axum::http::{HeaderMap, HeaderValue, Method};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::ClientPool;
use betterlite_roi::settings::{ROISettings, normalize_email};
use futures_util::Stream;
use futures_util::stream::try_unfold;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::ExposeSecret as _;
use serde::de::DeserializeOwned;
use tokio::time::sleep;
use url::Url;

use super::adapters::{RepositoryValue, repository_values};
use super::error::{BuildError, SourceError};
use super::models::{
    GitHubUserProfile, PAGINATION_ERROR, REPOSITORY_PAGE_ERROR, REPOSITORY_SEARCH_PAGES,
    RepositoryItem,
};
use super::pages::has_next_link;
use super::transport::Transport;

pub const ACCEPT_GITHUB_JSON: &str = "application/vnd.github+json";
pub const GITHUB_TIMEOUT: Duration = Duration::from_secs(45);
// B173's transport never follows redirects, so the GitHub client hands a 3xx answer back to its caller
pub const FOLLOW_REDIRECTS: bool = false;
// Python's quote(safe="") leaves only the unreserved characters alone; "/" and everything else is encoded
const QUOTE_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

pub struct GitHub<T: Transport> {
    pub transport: T,
    pub api_url: String,
    pub headers: HeaderMap,
    pub timeout: Duration,
    pub profiles: Mutex<BTreeMap<String, Option<String>>>,
}

impl<T: Transport> GitHub<T> {
    pub fn new(settings: &ROISettings, transport: T) -> Result<Self, SourceError> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static(ACCEPT_GITHUB_JSON));
        let token = settings.github_token.expose_secret();
        if !token.is_empty() {
            let mut value =
                HeaderValue::from_str(&format!("Bearer {token}")).map_err(SourceError::Token)?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        Ok(Self {
            transport,
            api_url: settings.github_api_url.trim_end_matches('/').to_owned(),
            headers,
            timeout: GITHUB_TIMEOUT,
            profiles: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn url(&self, path: &str) -> Result<Url, SourceError> {
        Url::parse(&format!(
            "{}/{}",
            self.api_url,
            path.trim_start_matches('/')
        ))
        .map_err(SourceError::Url)
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        params: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<UpstreamResponse, SourceError> {
        let url = self.url(path)?;
        let mut attempt: u32 = 0;
        loop {
            let outbound = OutboundRequest {
                method: method.clone(),
                url: url.clone(),
                query: params.to_vec(),
                headers: self.headers.clone(),
                body: body
                    .as_ref()
                    .map_or(RequestBody::Empty, |json| RequestBody::Json(json.clone())),
                deadline: self.timeout,
            };
            let response = self
                .transport
                .send(outbound)
                .await
                .map_err(SourceError::Transport)?;
            let status = response.status;
            if matches!(status, 429 | 502 | 503 | 504) && method == Method::GET && attempt < 2 {
                let delay = Duration::from_millis(500).saturating_mul(attempt.saturating_add(1));
                sleep(delay).await;
                attempt = attempt.saturating_add(1);
                continue;
            }
            if status < 400 {
                return Ok(response);
            }
            return Err(SourceError::Http {
                label: http_label(status),
                status,
            });
        }
    }

    pub async fn fetch_page<M>(
        &self,
        path: &str,
        params: &[(String, String)],
        page: i64,
        error_message: &str,
    ) -> Result<(Vec<M>, bool), SourceError>
    where
        M: DeserializeOwned,
    {
        let mut page_params = params.to_vec();
        page_params.push(("per_page".to_owned(), "100".to_owned()));
        page_params.push(("page".to_owned(), page.to_string()));
        let response = self.request(Method::GET, path, &page_params, None).await?;
        let items =
            serde_json::from_slice(&response.body).map_err(|error| SourceError::Pagination {
                message: error_message.to_owned(),
                source: error,
            })?;
        Ok((items, has_next_link(&response.headers)))
    }

    pub fn pages<'client, M>(
        &'client self,
        path: &'client str,
        params: &'client [(String, String)],
        limit: u32,
    ) -> impl Stream<Item = Result<Vec<M>, SourceError>> + Send + 'client
    where
        M: DeserializeOwned + 'client,
    {
        try_unfold(Some((1_u32, false)), move |state| async move {
            let Some((page, exhausted)) = state else {
                return Ok(None);
            };
            if exhausted || page > limit {
                return Err(SourceError::PaginationLimit);
            }
            let (items, has_next) = self
                .fetch_page::<M>(path, params, i64::from(page), PAGINATION_ERROR)
                .await?;
            let next = (has_next).then(|| (page.saturating_add(1), page >= limit));
            Ok(Some((items, next)))
        })
    }

    pub async fn repositories(
        &self,
        query: &str,
        page: i64,
    ) -> Result<(Vec<RepositoryValue>, bool), SourceError> {
        let params = vec![
            ("sort".to_owned(), "updated".to_owned()),
            ("direction".to_owned(), "desc".to_owned()),
            (
                "affiliation".to_owned(),
                "owner,collaborator,organization_member".to_owned(),
            ),
        ];
        if query.is_empty() {
            let (repositories, has_more) = self
                .fetch_page::<RepositoryItem>("user/repos", &params, page, REPOSITORY_PAGE_ERROR)
                .await?;
            return Ok((repository_values(&repositories), has_more));
        }
        self.search_repositories(query, page, &params).await
    }

    async fn search_repositories(
        &self,
        query: &str,
        page: i64,
        params: &[(String, String)],
    ) -> Result<(Vec<RepositoryValue>, bool), SourceError> {
        let normalized_query = caseless::default_case_fold_str(query);
        let mut github_page = page
            .saturating_sub(1)
            .saturating_mul(REPOSITORY_SEARCH_PAGES)
            .saturating_add(1);
        let mut pages_remaining = REPOSITORY_SEARCH_PAGES;
        let mut matches: Vec<RepositoryItem> = Vec::new();
        loop {
            let (repositories, has_more) = self
                .fetch_page::<RepositoryItem>(
                    "user/repos",
                    params,
                    github_page,
                    REPOSITORY_PAGE_ERROR,
                )
                .await?;
            matches.extend(repositories.into_iter().filter(|repository| {
                caseless::default_case_fold_str(&repository.full_name)
                    .contains(normalized_query.as_str())
            }));
            if pages_remaining <= 1 || !has_more {
                return Ok((repository_values(&matches), has_more));
            }
            github_page = github_page.saturating_add(1);
            pages_remaining = pages_remaining.saturating_sub(1);
        }
    }

    pub async fn test_repositories(&self, repos: &[String]) -> Result<(), SourceError> {
        for repo in repos {
            self.request(Method::GET, &format!("repos/{repo}"), &[], None)
                .await?;
            let params = [
                ("per_page".to_owned(), "1".to_owned()),
                ("state".to_owned(), "closed".to_owned()),
            ];
            self.request(Method::GET, &format!("repos/{repo}/pulls"), &params, None)
                .await?;
        }
        Ok(())
    }

    pub async fn profile_email(&self, login: &str, fallback: &str) -> String {
        let key = caseless::default_case_fold_str(login);
        let cached = self
            .profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        if let Some(address) = cached {
            return address.unwrap_or_else(|| fallback.to_owned());
        }
        let address = self.load_profile_email(login).await;
        self.profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, address.clone());
        address.unwrap_or_else(|| fallback.to_owned())
    }

    async fn load_profile_email(&self, login: &str) -> Option<String> {
        let encoded = utf8_percent_encode(login, QUOTE_SAFE);
        let url = self.url(&format!("users/{encoded}")).ok()?;
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: self.headers.clone(),
            body: RequestBody::Empty,
            deadline: self.timeout,
        };
        let response = self.transport.send(request).await.ok()?;
        if response.status != 200 {
            return None;
        }
        let profile: GitHubUserProfile = serde_json::from_slice(&response.body).ok()?;
        let email = profile.email.unwrap_or_default();
        Some(normalize_email(&email))
    }

    // B173's pool owns and drains the underlying clients, so this GitHub handle has nothing to close
    pub fn close(self) {}
}

const fn http_label(status: u16) -> &'static str {
    match status {
        401 => "Authentication failed. Check the configured GitHub token.",
        403 => {
            "GitHub denied access or reached a rate limit. Check token permissions and organization approval."
        }
        404 => {
            "GitHub repository or organization not found. Check its name, token access, and API URL."
        }
        429 => "GitHub rate limit reached. Wait before syncing again.",
        _ => "GitHub returned an error.",
    }
}

impl GitHub<ProxiedClient> {
    pub fn pooled(
        settings: &ROISettings,
        pool: &ClientPool,
        tls: &TlsPolicy,
        routing: &Arc<ProxyPolicy>,
    ) -> Result<Self, BuildError> {
        match pool.client(tls, routing) {
            Ok(client) => Self::new(settings, client).map_err(BuildError::Source),
            Err(error) => Err(BuildError::Pool(error)),
        }
    }
}
