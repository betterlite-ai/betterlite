use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::time::{Instant, sleep};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::cyberark::Clock;
use crate::vault::HashicorpSecretManager;
use crate::vault::transport::{VaultMethod, VaultRequest, VaultResponse, VaultTransport};

#[derive(Debug, Error)]
#[error("connection refused")]
pub(super) struct Refused;

#[derive(Clone)]
pub(super) enum Reply {
    Answer {
        status: u16,
        body: String,
    },
    Failure,
    Slow {
        delay: Duration,
        status: u16,
        body: String,
    },
}

pub(super) fn answered(status: u16, body: &str) -> Reply {
    Reply::Answer {
        status,
        body: body.to_owned(),
    }
}

pub(super) fn slow(status: u16, body: &str, delay: Duration) -> Reply {
    Reply::Slow {
        delay,
        status,
        body: body.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Recorded {
    pub(super) method: VaultMethod,
    pub(super) url: String,
    pub(super) token: Option<String>,
    pub(super) namespace: Option<String>,
    pub(super) body: Option<String>,
    pub(super) timeout: Duration,
    pub(super) follow_redirects: bool,
    pub(super) max_response_bytes: usize,
    pub(super) blocking: bool,
}

pub(super) struct FakeTransport {
    routes: Mutex<BTreeMap<String, VecDeque<Reply>>>,
    requests: Mutex<Vec<Recorded>>,
}

impl FakeTransport {
    pub(super) fn new(routes: &[(&str, Vec<Reply>)]) -> Arc<Self> {
        Arc::new(Self {
            routes: Mutex::new(
                routes
                    .iter()
                    .map(|(route, replies)| ((*route).to_owned(), VecDeque::from(replies.clone())))
                    .collect(),
            ),
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(&self, request: &VaultRequest, blocking: bool) -> Reply {
        let route = format!("{} {}", method_name(request.method), request.url);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                method: request.method,
                url: request.url.clone(),
                token: request
                    .token
                    .as_ref()
                    .map(|value| value.expose_secret().to_owned()),
                namespace: request.namespace.clone(),
                body: request
                    .body
                    .as_ref()
                    .map(|value| value.expose_secret().to_owned()),
                timeout: request.timeout,
                follow_redirects: request.follow_redirects,
                max_response_bytes: request.max_response_bytes,
                blocking,
            });
        self.routes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&route)
            .and_then(|replies| {
                if replies.len() > 1 {
                    replies.pop_front()
                } else {
                    replies.front().cloned()
                }
            })
            .unwrap_or(Reply::Answer {
                status: 500,
                body: String::new(),
            })
    }
}

pub(super) const fn method_name(method: VaultMethod) -> &'static str {
    match method {
        VaultMethod::Get => "GET",
        VaultMethod::Post => "POST",
        VaultMethod::Delete => "DELETE",
    }
}

fn into_response(reply: &Reply) -> Result<VaultResponse, Refused> {
    match reply {
        Reply::Answer { status, body } | Reply::Slow { status, body, .. } => Ok(VaultResponse {
            status: *status,
            body: SecretString::from(body.as_str()),
        }),
        Reply::Failure => Err(Refused),
    }
}

impl VaultTransport for Arc<FakeTransport> {
    type Error = Refused;

    fn send(
        &self,
        request: VaultRequest,
    ) -> impl Future<Output = Result<VaultResponse, Refused>> + Send {
        let reply = self.answer(&request, false);
        async move {
            match reply {
                Reply::Slow { delay, .. } => {
                    sleep(delay).await;
                    into_response(&reply)
                }
                Reply::Answer { .. } | Reply::Failure => into_response(&reply),
            }
        }
    }

    fn send_blocking(&self, request: VaultRequest) -> Result<VaultResponse, Refused> {
        into_response(&self.answer(&request, true))
    }
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) type Scripted = (
    HashicorpSecretManager<Arc<FakeTransport>>,
    Arc<FakeTransport>,
);

pub(super) fn scripted(pairs: &[(&str, &str)], routes: &[(&str, Vec<Reply>)]) -> Scripted {
    let transport = FakeTransport::new(routes);
    let clock: Clock = Arc::new(Instant::now);
    let manager = HashicorpSecretManager::new(&environment(pairs), Arc::clone(&transport), clock)
        .expect("the test environment configures Vault");
    (manager, transport)
}
