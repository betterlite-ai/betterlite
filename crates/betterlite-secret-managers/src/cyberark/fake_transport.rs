#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::time::Instant;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::CyberArkSecretManager;
use super::transport::{
    ClientCertificate, ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod,
};

pub(super) const AUTH_ROUTE: &str = "POST http://127.0.0.1:8080/authn/default/admin/authenticate";
pub(super) const TOKEN_JSON: &str = "{\"protected\":\"abc\"}";
pub(super) const TOKEN_HEADER: &str = "Token token=\"eyJwcm90ZWN0ZWQiOiJhYmMifQ==\"";
pub(super) const POLICY_ROUTE: &str = "POST http://127.0.0.1:8080/policies/default/policy/root";

#[derive(Debug, Error)]
#[error("connection refused")]
pub(super) struct FakeError;

#[derive(Clone)]
pub(super) enum Reply {
    Status(u16, &'static str),
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Recorded {
    pub(super) route: String,
    pub(super) authorization: Option<String>,
    pub(super) content_type: Option<&'static str>,
    pub(super) body: Option<String>,
    pub(super) verify_tls: bool,
    pub(super) client_certificate: Option<ClientCertificate>,
    pub(super) blocking: bool,
    pub(super) at: Duration,
}

pub(super) struct FakeTransport {
    routes: Mutex<BTreeMap<String, VecDeque<Reply>>>,
    requests: Mutex<Vec<Recorded>>,
    origin: Instant,
}

impl FakeTransport {
    pub(super) fn new(routes: &[(&str, Vec<Reply>)]) -> Arc<Self> {
        Arc::new(Self {
            routes: Mutex::new(
                routes
                    .iter()
                    .map(|(route, replies)| {
                        ((*route).to_owned(), replies.iter().cloned().collect())
                    })
                    .collect(),
            ),
            requests: Mutex::new(Vec::new()),
            origin: Instant::now(),
        })
    }

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn routes_hit(&self) -> Vec<String> {
        self.recorded()
            .iter()
            .map(|seen| seen.route.clone())
            .collect()
    }

    fn answer(&self, request: &ConjurRequest<'_>, blocking: bool) -> Reply {
        let verb = match request.method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
        };
        let route = format!("{verb} {}", request.url);
        self.requests.lock().unwrap().push(Recorded {
            route: route.clone(),
            authorization: request
                .authorization
                .map(|header| header.expose_secret().to_owned()),
            content_type: request.content_type,
            body: request.body.map(|body| body.expose_secret().to_owned()),
            verify_tls: request.verify_tls,
            client_certificate: request.client_certificate.cloned(),
            blocking,
            at: self.origin.elapsed(),
        });
        let reply = self
            .routes
            .lock()
            .unwrap()
            .get_mut(&route)
            .and_then(|replies| {
                if replies.len() > 1 {
                    replies.pop_front()
                } else {
                    replies.front().cloned()
                }
            });
        reply.unwrap_or(Reply::Status(500, ""))
    }
}

fn into_response(reply: &Reply) -> Result<ConjurResponse, FakeError> {
    match reply {
        Reply::Status(status, body) => Ok(ConjurResponse {
            status: *status,
            body: SecretString::from(*body),
        }),
        Reply::Failure => Err(FakeError),
    }
}

impl ConjurTransport for Arc<FakeTransport> {
    type Error = FakeError;

    fn send<'borrowed>(
        &'borrowed self,
        request: ConjurRequest<'borrowed>,
    ) -> impl Future<Output = Result<ConjurResponse, FakeError>> + Send {
        ready(into_response(&self.answer(&request, false)))
    }

    fn send_blocking(&self, request: ConjurRequest<'_>) -> Result<ConjurResponse, FakeError> {
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
    CyberArkSecretManager<Arc<FakeTransport>>,
    Arc<FakeTransport>,
);

pub(super) fn scripted(pairs: &[(&str, &str)], routes: &[(&str, Vec<Reply>)]) -> Scripted {
    let transport = FakeTransport::new(routes);
    let manager = CyberArkSecretManager::new(&environment(pairs), Arc::clone(&transport)).unwrap();
    (manager, transport)
}

pub(super) fn keyed(routes: &[(&str, Vec<Reply>)]) -> Scripted {
    let mut routes_with_login = vec![(AUTH_ROUTE, vec![Reply::Status(200, TOKEN_JSON)])];
    routes_with_login.extend(routes.iter().cloned());
    scripted(&[("CYBERARK_API_KEY", "key")], &routes_with_login)
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
