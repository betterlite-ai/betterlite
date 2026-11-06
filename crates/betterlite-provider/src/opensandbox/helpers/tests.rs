#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_models::sandbox::{ContainerHandle, HiddenParams};
use futures_util::StreamExt as _;
use futures_util::stream;
use secrecy::SecretString;
use tokio::runtime::Builder;

use super::{OpenSandbox, OpenSandboxError, PollClock, SandboxHttp, SandboxRequest, SandboxStream};

mod corpus;
mod create;
mod defaults;
mod poll;
mod reply;
mod request;
mod sse;

pub(super) type Environment = BTreeMap<String, String>;

#[derive(Clone)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
}

pub(super) struct FakeHttp {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<SandboxRequest>>,
}

impl FakeHttp {
    pub(super) fn new(replies: &[(u16, &str)]) -> Self {
        Self::from_replies(
            replies
                .iter()
                .map(|(status, body)| Reply {
                    status: *status,
                    body: body.as_bytes().to_vec(),
                })
                .collect(),
        )
    }

    pub(super) fn from_replies(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<SandboxRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl SandboxHttp for FakeHttp {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, OpenSandboxError> {
        self.requests.lock().unwrap().push(request);
        let reply = {
            let mut replies = self.replies.lock().unwrap();
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

pub(super) struct ManualClock {
    now: Mutex<Duration>,
}

impl ManualClock {
    pub(super) fn new() -> Self {
        Self {
            now: Mutex::new(Duration::ZERO),
        }
    }

    pub(super) fn elapsed(&self) -> Duration {
        *self.now.lock().unwrap()
    }
}

#[async_trait]
impl PollClock for ManualClock {
    fn now(&self) -> Duration {
        self.elapsed()
    }

    async fn sleep(&self, duration: Duration) {
        let mut now = self.now.lock().unwrap();
        let next = now.saturating_add(duration);
        *now = next;
    }
}

pub(super) fn environment(entries: &[(&str, &str)]) -> Environment {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn handle(api_key: Option<&str>) -> ContainerHandle {
    ContainerHandle {
        id: "sbx_1".to_owned(),
        provider: "opensandbox".to_owned(),
        domain: Some("http://sandbox.example".to_owned()),
        hidden_params: HiddenParams {
            envd_access_token: None,
            traffic_access_token: None,
            api_key: api_key.map(SecretString::from),
            api_base: None,
        },
    }
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

pub(super) fn sandbox(
    http: FakeHttp,
    environment: Environment,
) -> OpenSandbox<FakeHttp, Environment, ManualClock> {
    OpenSandbox {
        http,
        environment,
        clock: ManualClock::new(),
    }
}
