use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_secret_managers::cyberark::transport::{
    ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::Instant;

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum Reply {
    Failure { failure: bool },
    Answer { status: u16, body: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Route {
    route: String,
    replies: Vec<Reply>,
}

#[derive(Debug, Error)]
#[error("connection refused")]
pub(super) struct Refused;

pub(super) type Requests = Arc<Mutex<Vec<Value>>>;

pub(super) struct Script {
    replies: Mutex<BTreeMap<String, Vec<Reply>>>,
    requests: Requests,
    origin: Instant,
}

impl Script {
    pub(super) fn new(routes: &[Route], requests: Requests) -> Self {
        let replies = routes
            .iter()
            .map(|entry| {
                let copies = entry.replies.clone();
                (entry.route.clone(), copies)
            })
            .collect();
        Self {
            replies: Mutex::new(replies),
            requests,
            origin: Instant::now(),
        }
    }

    fn answer(
        &self,
        request: &ConjurRequest<'_>,
        blocking: bool,
    ) -> Result<ConjurResponse, Refused> {
        let verb = match request.method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
        };
        let route = format!("{verb} {}", request.url);
        let elapsed = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        let certificate = request
            .client_certificate
            .map(|pair| json!({"certificate": pair.certificate_path, "key": pair.key_path}));
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "route": route,
                "authorization": request.authorization.map(|header| header.expose_secret().to_owned()),
                "content_type": request.content_type,
                "body": request.body.map(|body| body.expose_secret().to_owned()),
                "verify_tls": request.verify_tls,
                "client_certificate": certificate,
                "blocking": (!request.url.ends_with("/authenticate")).then_some(blocking),
                "at_ms": elapsed,
            }));
        match self.next_reply(&route) {
            Some(Reply::Failure { failure: true }) => Err(Refused),
            Some(Reply::Failure { failure: false }) | None => Ok(response(500, "")),
            Some(Reply::Answer { status, body }) => Ok(response(status, &body)),
        }
    }

    fn next_reply(&self, route: &str) -> Option<Reply> {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(route)
            .and_then(|replies| {
                if replies.len() > 1 {
                    Some(replies.remove(0))
                } else {
                    replies.first().cloned()
                }
            })
    }
}

fn response(status: u16, body: &str) -> ConjurResponse {
    ConjurResponse {
        status,
        body: SecretString::from(body),
    }
}

impl ConjurTransport for Script {
    type Error = Refused;

    fn send<'borrowed>(
        &'borrowed self,
        request: ConjurRequest<'borrowed>,
    ) -> impl Future<Output = Result<ConjurResponse, Refused>> + Send {
        ready(self.answer(&request, false))
    }

    fn send_blocking(&self, request: ConjurRequest<'_>) -> Result<ConjurResponse, Refused> {
        self.answer(&request, true)
    }
}
