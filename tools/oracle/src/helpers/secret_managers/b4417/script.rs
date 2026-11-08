use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_secret_managers::vault::transport::{
    VaultMethod, VaultRequest, VaultResponse, VaultTransport,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::sleep;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Route {
    route: String,
    replies: Vec<Reply>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum Reply {
    Failure { failure: bool },
    Timeout { timeout: bool },
    Answer { status: u16, body: String },
}

#[derive(Debug, Error)]
#[error("connection refused")]
pub(super) struct Refused;

pub(super) type Requests = Arc<Mutex<Vec<Value>>>;

pub(super) struct Script {
    replies: Mutex<BTreeMap<String, Vec<Reply>>>,
    requests: Requests,
}

impl Script {
    pub(super) fn new(routes: &[Route], requests: Requests) -> Self {
        let replies = routes
            .iter()
            .map(|entry| (entry.route.clone(), entry.replies.clone()))
            .collect();
        Self {
            replies: Mutex::new(replies),
            requests,
        }
    }

    fn answer(&self, request: &VaultRequest) -> Reply {
        let verb = match request.method {
            VaultMethod::Get => "GET",
            VaultMethod::Post => "POST",
            VaultMethod::Delete => "DELETE",
        };
        let body = request
            .body
            .as_ref()
            .map(|body| serde_json::from_str::<Value>(body.expose_secret()).unwrap_or(Value::Null));
        let certificate = request
            .client_certificate
            .as_ref()
            .map(|pair| json!({"certificate": pair.certificate_path, "key": pair.key_path}));
        let route = format!("{verb} {}", request.url);
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "method": verb,
                "url": request.url,
                "namespace": request.namespace,
                "body": body,
                "client_certificate": certificate,
            }));
        self.next_reply(&route).unwrap_or_else(|| Reply::Answer {
            status: 500,
            body: String::new(),
        })
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

fn response(status: u16, body: &str) -> VaultResponse {
    VaultResponse {
        status,
        body: SecretString::from(body),
    }
}

impl VaultTransport for Script {
    type Error = Refused;

    fn send(
        &self,
        request: VaultRequest,
    ) -> impl Future<Output = Result<VaultResponse, Refused>> + Send {
        let timeout = request.timeout;
        let reply = self.answer(&request);
        async move {
            match reply {
                Reply::Failure { failure: true } => Err(Refused),
                Reply::Failure { failure: false } | Reply::Timeout { timeout: false } => {
                    Ok(response(500, ""))
                }
                Reply::Timeout { timeout: true } => {
                    sleep(timeout.saturating_add(Duration::from_secs(1))).await;
                    Ok(response(500, ""))
                }
                Reply::Answer { status, body } => Ok(response(status, &body)),
            }
        }
    }

    fn send_blocking(&self, request: VaultRequest) -> Result<VaultResponse, Refused> {
        match self.answer(&request) {
            Reply::Failure { failure: true } => Err(Refused),
            Reply::Failure { failure: false } | Reply::Timeout { timeout: false } => {
                Ok(response(500, ""))
            }
            Reply::Timeout { timeout: true } => Ok(response(500, "")),
            Reply::Answer { status, body } => Ok(response(status, &body)),
        }
    }
}
