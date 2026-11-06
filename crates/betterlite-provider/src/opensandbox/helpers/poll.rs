use std::collections::BTreeMap;
use std::time::Duration;

use http::{HeaderMap, Method};
use serde::Deserialize;
use serde_json::value::RawValue;
use url::Url;

use super::constants::EXECD_PORT;
use super::reply::{check_status, collect_reply};
use super::request::as_str_dict;
use super::{OpenSandbox, OpenSandboxError, PollClock, SandboxHttp, SandboxRequest};

#[derive(Debug, Clone, Copy)]
pub struct GetEndpoint<'request> {
    pub sandbox_id: &'request str,
    pub api_base: &'request str,
    pub headers: &'request HeaderMap,
    pub use_server_proxy: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct WaitForEndpoint<'request> {
    pub endpoint: GetEndpoint<'request>,
    pub ready_timeout: Duration,
    pub poll_interval: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct WaitUntilRunning<'request> {
    pub sandbox_id: &'request str,
    pub api_base: &'request str,
    pub headers: &'request HeaderMap,
    pub ready_timeout: Duration,
    pub poll_interval: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct SandboxDocument {
    #[serde(default)]
    status: Option<SandboxStatus>,
}

#[derive(Debug, Deserialize)]
struct SandboxStatus {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EndpointDocument {
    #[serde(default)]
    endpoint: Option<Box<RawValue>>,
    #[serde(default)]
    headers: Option<BTreeMap<String, Box<RawValue>>>,
}

#[must_use]
pub fn sandbox_state(document: &RawValue) -> Option<String> {
    let Ok(parsed) = serde_json::from_str::<SandboxDocument>(document.get()) else {
        return None;
    };
    parsed.status?.state
}

impl<Http, Environment, Clock> OpenSandbox<Http, Environment, Clock>
where
    Http: SandboxHttp,
    Environment: Sync,
    Clock: PollClock,
{
    pub async fn wait_until_running(
        &self,
        request: &WaitUntilRunning<'_>,
        client: Option<&Http>,
    ) -> Result<(), OpenSandboxError> {
        let deadline = self.clock.now().saturating_add(request.ready_timeout);
        loop {
            match self.poll_state(request, client).await? {
                PollState::Running => return Ok(()),
                PollState::Terminal(state) => {
                    return Err(OpenSandboxError::UnexpectedState {
                        sandbox_id: request.sandbox_id.to_owned(),
                        state,
                    });
                }
                PollState::Waiting => {}
            }
            let remaining = deadline.checked_sub(self.clock.now());
            if remaining.is_none_or(|left| left.is_zero()) {
                return Err(OpenSandboxError::NotRunning {
                    sandbox_id: request.sandbox_id.to_owned(),
                    timeout: request.ready_timeout,
                });
            }
            self.clock.sleep(request.poll_interval).await;
        }
    }

    async fn poll_state(
        &self,
        request: &WaitUntilRunning<'_>,
        client: Option<&Http>,
    ) -> Result<PollState, OpenSandboxError> {
        let url = Url::parse(&format!(
            "{}/sandboxes/{}",
            request.api_base, request.sandbox_id
        ))
        .map_err(|source| OpenSandboxError::Url { source })?;
        let body = self.get_body(url, request.headers, client).await?;
        let document: Box<RawValue> =
            serde_json::from_slice(&body).map_err(|source| OpenSandboxError::Decode { source })?;
        match sandbox_state(&document).as_deref() {
            Some("Running") => Ok(PollState::Running),
            Some(terminal @ ("Failed" | "Stopping" | "Terminated")) => {
                Ok(PollState::Terminal(terminal.to_owned()))
            }
            _ => Ok(PollState::Waiting),
        }
    }

    pub async fn wait_for_execd_endpoint(
        &self,
        request: WaitForEndpoint<'_>,
        client: Option<&Http>,
    ) -> Result<Endpoint, OpenSandboxError> {
        let expires = request.ready_timeout.saturating_add(self.clock.now());
        let mut failure: Option<Box<OpenSandboxError>>;
        loop {
            let error = match self.get_execd_endpoint(request.endpoint, client).await {
                Ok(endpoint) => return Ok(endpoint),
                Err(error) => error,
            };
            if is_retryable(&error) {
                failure = Some(Box::new(error));
            } else {
                return Err(error);
            }
            if expires.saturating_sub(self.clock.now()).is_zero() {
                return Err(endpoint_timeout(
                    request.endpoint.sandbox_id,
                    request.ready_timeout,
                    failure,
                ));
            }
            let interval = request.poll_interval;
            self.clock.sleep(interval).await;
        }
    }

    pub async fn get_execd_endpoint(
        &self,
        request: GetEndpoint<'_>,
        client: Option<&Http>,
    ) -> Result<Endpoint, OpenSandboxError> {
        let mut url = Url::parse(&format!(
            "{}/sandboxes/{}/endpoints/{}",
            request.api_base, request.sandbox_id, EXECD_PORT
        ))
        .map_err(|source| OpenSandboxError::Url { source })?;
        url.query_pairs_mut().append_pair(
            "use_server_proxy",
            if request.use_server_proxy {
                "true"
            } else {
                "false"
            },
        );
        let body = self.get_body(url, request.headers, client).await?;
        let document: EndpointDocument =
            serde_json::from_slice(&body).map_err(|source| OpenSandboxError::Decode { source })?;
        let endpoint = document
            .endpoint
            .as_deref()
            .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
            .filter(|endpoint| !endpoint.is_empty())
            .ok_or_else(|| OpenSandboxError::MissingEndpoint {
                sandbox_id: request.sandbox_id.to_owned(),
            })?;
        Ok(Endpoint {
            url: endpoint,
            headers: as_str_dict(document.headers.as_ref()),
        })
    }

    async fn get_body(
        &self,
        url: Url,
        headers: &HeaderMap,
        client: Option<&Http>,
    ) -> Result<Vec<u8>, OpenSandboxError> {
        let response = client
            .unwrap_or(&self.http)
            .send(get_request(url, headers))
            .await?;
        check_status(response.status)?;
        collect_reply(response.body).await
    }
}

enum PollState {
    Running,
    Terminal(String),
    Waiting,
}

fn endpoint_timeout(
    sandbox_id: &str,
    timeout: Duration,
    last: Option<Box<OpenSandboxError>>,
) -> OpenSandboxError {
    OpenSandboxError::EndpointNotReady {
        sandbox_id: sandbox_id.to_owned(),
        timeout,
        last,
    }
}

const fn is_retryable(error: &OpenSandboxError) -> bool {
    if matches!(error, OpenSandboxError::HttpStatus { status: 404 }) {
        return true;
    }
    matches!(error, OpenSandboxError::MissingEndpoint { .. })
}

fn get_request(url: Url, headers: &HeaderMap) -> SandboxRequest {
    SandboxRequest {
        method: Method::GET,
        url,
        headers: headers.clone(),
        body: Vec::new(),
        timeout: None,
    }
}
