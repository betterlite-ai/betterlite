use std::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use subtle::ConstantTimeEq as _;
use url::Url;

use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::http_analytics_loggers::delivery::{DELIVERY_DEADLINE, Delivery, PostRequest};
use crate::langsmith_payloads::prepare::RunData;

pub const CONTENT_TYPE_JSON: &str = "application/json";
pub const API_KEY_HEADER: &str = "x-api-key";
pub const TENANT_ID_HEADER: &str = "x-tenant-id";
pub const RUNS_BATCH_ENDPOINT: &str = "runs/batch";
pub const API_VERSION: &str = "/api/v1";

#[derive(Clone, Debug)]
pub struct QueuedRun {
    pub data: RunData,
    pub api_key: Option<SecretString>,
    pub project: String,
    pub base_url: String,
    pub tenant_id: Option<String>,
}

struct Group {
    api_key: SecretString,
    project: String,
    base_url: String,
    tenant_id: Option<String>,
    runs: Vec<RunData>,
}

#[derive(Debug, thiserror::Error)]
pub enum HeaderError {
    #[error("the LangSmith API key cannot be a header value")]
    ApiKey(#[source] InvalidHeaderValue),
    #[error("the LangSmith tenant id cannot be a header value")]
    TenantId(#[source] InvalidHeaderValue),
}

#[derive(Serialize)]
struct RunBatch<'runs> {
    post: Vec<&'runs RunData>,
}

// LangsmithLogger._add_endpoint_to_url: the version is only appended when the base URL lacks it
#[must_use]
pub fn endpoint_url(base_url: &str) -> String {
    let mut url = if base_url.contains(API_VERSION) {
        base_url.to_owned()
    } else {
        format!("{}{API_VERSION}", base_url.trim_end_matches('/'))
    };
    if !url.ends_with('/') {
        url.push('/');
    }
    url.push_str(RUNS_BATCH_ENDPOINT);
    url
}

pub struct LangsmithBatch<D> {
    pub delivery: Arc<D>,
}

impl<D> BatchDelivery<QueuedRun> for LangsmithBatch<D>
where
    D: Delivery + 'static,
{
    async fn deliver(
        &self,
        batch: &[QueuedRun],
        settled: &AtomicUsize,
    ) -> Result<Vec<QueuedRun>, BatchCancelled<QueuedRun>> {
        for group in group_by_credentials(batch) {
            self.post_group(&group).await;
        }
        // LiteLLM drops a failed group rather than retrying it, so every queued run is settled here
        settled.fetch_add(batch.len(), Ordering::SeqCst);
        Ok(Vec::new())
    }
}

impl<D> LangsmithBatch<D>
where
    D: Delivery + 'static,
{
    async fn post_group(&self, group: &Group) {
        let Some(request) = group.request() else {
            return;
        };
        match self.delivery.post(request).await {
            Ok(response) if response.status >= 300 => {
                tracing::error!(
                    status = response.status,
                    detail = str::from_utf8(&response.body).unwrap_or_default(),
                    "Langsmith HTTP Error"
                );
            }
            Ok(_response) => {
                tracing::debug!(
                    runs = group.runs.len(),
                    "Batch of runs successfully created"
                );
            }
            Err(error) => {
                tracing::error!(%error, "Langsmith Layer Error - the request failed");
            }
        }
    }
}

// LangsmithLogger._group_batches_by_credentials, in the first-seen order Python's dict keeps
fn group_by_credentials(batch: &[QueuedRun]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for queued in batch {
        let Some(api_key) = queued.api_key.as_ref() else {
            tracing::warn!(
                project = %queued.project,
                "Langsmith Logging - credentials missing; the run is skipped"
            );
            continue;
        };
        let existing = groups
            .iter_mut()
            .find(|group| group.shares_credentials_with(queued));
        match existing {
            Some(group) => group.runs.push(queued.data.clone()),
            None => groups.push(Group {
                api_key: api_key.clone(),
                project: queued.project.clone(),
                base_url: queued.base_url.clone(),
                tenant_id: queued.tenant_id.clone(),
                runs: vec![queued.data.clone()],
            }),
        }
    }
    groups
}

impl Group {
    fn shares_credentials_with(&self, queued: &QueuedRun) -> bool {
        let Some(key) = queued.api_key.as_ref() else {
            return false;
        };
        let keys_match = bool::from(
            self.api_key
                .expose_secret()
                .as_bytes()
                .ct_eq(key.expose_secret().as_bytes()),
        );
        keys_match
            && self.project == queued.project
            && self.base_url == queued.base_url
            && self.tenant_id == queued.tenant_id
    }

    // LangsmithLogger._log_batch_on_langsmith builds the URL, the headers and the run batch before it posts
    fn request(&self) -> Option<PostRequest> {
        let url = match Url::parse(&endpoint_url(&self.base_url)) {
            Ok(url) => url,
            Err(source) => {
                tracing::error!(
                    %source,
                    base_url = %self.base_url,
                    "Langsmith Layer Error - the base URL is not valid"
                );
                return None;
            }
        };
        let headers = match headers(&self.api_key, self.tenant_id.as_deref()) {
            Ok(headers) => headers,
            Err(error) => {
                tracing::error!(%error, "Langsmith Layer Error - the credentials cannot be sent");
                return None;
            }
        };
        let body = match serde_json::to_vec(&RunBatch {
            post: self.runs.iter().collect(),
        }) {
            Ok(body) => body,
            Err(source) => {
                tracing::error!(%source, "Langsmith Layer Error - the batch could not be encoded");
                return None;
            }
        };
        Some(PostRequest {
            url,
            headers,
            body,
            deadline: DELIVERY_DEADLINE,
        })
    }
}

// LangsmithLogger._log_batch_on_langsmith builds the same three headers
fn headers(api_key: &SecretString, tenant_id: Option<&str>) -> Result<HeaderMap, HeaderError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_JSON));
    let mut key = HeaderValue::try_from(api_key.expose_secret()).map_err(HeaderError::ApiKey)?;
    key.set_sensitive(true);
    headers.insert(HeaderName::from_static(API_KEY_HEADER), key);
    if let Some(tenant) = tenant_id {
        let value = HeaderValue::try_from(tenant).map_err(HeaderError::TenantId)?;
        headers.insert(HeaderName::from_static(TENANT_ID_HEADER), value);
    }
    Ok(headers)
}
