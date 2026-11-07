#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::time::Instant;

use super::HashicorpSecretManager;
use super::transport::{VaultMethod, VaultRequest, VaultResponse, VaultTransport};
use crate::cyberark::Clock;
use crate::cyberark::transport::ClientCertificate;

pub(super) const APPROLE_ROUTE: &str = "POST http://127.0.0.1:8200/v1/auth/approle/login";
pub(super) const TLS_ROUTE: &str = "POST http://127.0.0.1:8200/v1/auth/cert/login";
pub(super) const LOGIN_BODY: &str =
    "{\"auth\":{\"client_token\":\"hvs.first\",\"lease_duration\":3600}}";
pub(super) const SECOND_LOGIN_BODY: &str =
    "{\"auth\":{\"client_token\":\"hvs.second\",\"lease_duration\":3600}}";

#[derive(Debug, Error)]
#[error("connection refused")]
pub(super) struct Refused;

#[derive(Clone)]
pub(super) enum Reply {
    Answer { status: u16, body: String },
    Failure,
}

pub(super) fn answered(status: u16, body: &str) -> Reply {
    Reply::Answer {
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
    pub(super) client_certificate: Option<ClientCertificate>,
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
                client_certificate: request.client_certificate.clone(),
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
        Reply::Answer { status, body } => Ok(VaultResponse {
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
        ready(into_response(&self.answer(&request, false)))
    }

    fn send_blocking(&self, request: VaultRequest) -> Result<VaultResponse, Refused> {
        into_response(&self.answer(&request, true))
    }
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
