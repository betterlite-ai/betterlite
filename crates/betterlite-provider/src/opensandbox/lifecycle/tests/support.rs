use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::sandbox::{ContainerHandle, HiddenParams};
use futures_util::StreamExt as _;
use futures_util::stream;
use secrecy::SecretString;
use tokio::runtime::Builder;

use super::super::super::helpers::poll::Endpoint;
use super::super::super::helpers::{
    OpenSandbox, OpenSandboxError, PollClock, SandboxHttp, SandboxRequest, SandboxStream,
};
use super::super::{ContainerRef, CreateSandboxRequest, DeleteSandbox, RunCode, SandboxHandle};

pub(super) type Environment = BTreeMap<String, String>;
pub(super) type Sandbox = OpenSandbox<ScriptedHttp, Environment, StepClock>;

#[derive(Clone)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
}

pub(super) struct ScriptedHttp {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<SandboxRequest>>,
}

impl ScriptedHttp {
    pub(super) fn new(replies: &[(u16, &str)]) -> Self {
        Self {
            replies: Mutex::new(
                replies
                    .iter()
                    .map(|(status, body)| Reply {
                        status: *status,
                        body: body.as_bytes().to_vec(),
                    })
                    .collect(),
            ),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<SandboxRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn request(&self, index: usize) -> SandboxRequest {
        self.requests()[index].clone()
    }
}

#[async_trait]
impl SandboxHttp for ScriptedHttp {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, OpenSandboxError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let reply = {
            let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
            match replies.len() {
                0 => Reply {
                    status: 200,
                    body: Vec::new(),
                },
                1 => replies.front().cloned().unwrap(),
                _ => replies.pop_front().unwrap(),
            }
        };
        Ok(SandboxStream {
            status: reply.status,
            body: stream::iter(vec![Ok(reply.body)]).boxed(),
        })
    }
}

pub(super) struct StepClock {
    now: Mutex<Duration>,
    sleeps: Mutex<Vec<Duration>>,
}

impl StepClock {
    pub(super) fn new() -> Self {
        Self {
            now: Mutex::new(Duration::ZERO),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn sleeps(&self) -> Vec<Duration> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl PollClock for StepClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn sleep(&self, duration: Duration) {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(duration);
    }
}

pub(super) fn sandbox(replies: &[(u16, &str)], environment: Environment) -> Sandbox {
    OpenSandbox {
        http: ScriptedHttp::new(replies),
        environment,
        clock: StepClock::new(),
    }
}

pub(super) fn environment(entries: &[(&str, &str)]) -> Environment {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn block_on<Output, Work>(work: Work) -> Output
where
    Work: Future<Output = Output>,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(work)
}

pub(super) fn create_request<'request>(
    api_base: &'request str,
    api_key: &'request SecretString,
) -> CreateSandboxRequest<'request> {
    CreateSandboxRequest {
        template: None,
        timeout: None,
        allow_internet_access: None,
        api_key: Some(api_key),
        api_base: Some(api_base),
        metadata: None,
        env_vars: None,
        resource_limits: None,
        resource_requests: None,
        entrypoint: None,
        network_policy: None,
        secure_access: false,
        use_server_proxy: false,
        ready_timeout: None,
        poll_interval: None,
    }
}

pub(super) fn plain_handle(api_base: Option<&str>, api_key: Option<&str>) -> ContainerHandle {
    ContainerHandle {
        id: "sbx_1".to_owned(),
        provider: "opensandbox".to_owned(),
        domain: None,
        hidden_params: HiddenParams {
            envd_access_token: None,
            traffic_access_token: None,
            api_key: api_key.map(SecretString::from),
            api_base: api_base.map(str::to_owned),
        },
    }
}

pub(super) fn cached_handle(api_base: &str, api_key: &str, endpoint: &str) -> SandboxHandle {
    SandboxHandle {
        container: plain_handle(Some(api_base), Some(api_key)),
        execd_endpoint: Endpoint {
            url: endpoint.to_owned(),
            headers: BTreeMap::from([("X-Execd-Token".to_owned(), "execd-token".to_owned())]),
        },
        use_server_proxy: true,
    }
}

pub(super) fn run_request<'request>(
    container: ContainerRef<'request>,
    code: &'request str,
) -> RunCode<'request> {
    RunCode {
        container,
        code,
        api_key: None,
        api_base: None,
        language: None,
        use_server_proxy: false,
        ready_timeout: None,
        poll_interval: None,
    }
}

pub(super) fn delete_request(container: ContainerRef<'_>) -> DeleteSandbox<'_> {
    DeleteSandbox {
        container,
        api_key: None,
        api_base: None,
    }
}

pub(super) fn header<'request>(
    request: &'request SandboxRequest,
    name: &str,
) -> Option<&'request str> {
    let value = request.headers.get(name)?;
    value.to_str().ok()
}
