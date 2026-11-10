use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_azure::azure_client_secret::transport::{
    Clock, HttpMethod, HttpReply, HttpRequest, Transport,
};
use betterlite_azure::azure_client_secret::{ClientSecretInput, SecretResolver};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

pub(super) const NOW: i64 = 1_700_000_000;

#[derive(Clone, Deserialize)]
pub(super) struct Reply {
    #[serde(default)]
    pub(super) url_contains: Option<String>,
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(default)]
    pub(super) error: Option<String>,
}

pub(super) struct FixtureTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FixtureTransport {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for FixtureTransport {
    fn send(&self, request: &HttpRequest) -> io::Result<HttpReply> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let reply = {
            let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
            let index = replies
                .iter()
                .position(|reply| {
                    reply
                        .url_contains
                        .as_ref()
                        .is_none_or(|needle| request.url.contains(needle))
                })
                .ok_or_else(|| io::Error::other(format!("no reply for {}", request.url)))?;
            replies
                .remove(index)
                .ok_or_else(|| io::Error::other("no reply"))?
        };
        match reply.error {
            Some(message) => Err(io::Error::other(message)),
            None => Ok(HttpReply {
                status: reply.status,
                body: SecretString::from(reply.body),
            }),
        }
    }
}

pub(super) struct FixtureClock {
    pub(super) now: AtomicI64,
}

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

pub(super) struct FixtureResolver {
    pub(super) environment: BTreeMap<String, String>,
}

impl From<&super::steps::Step> for FixtureResolver {
    fn from(step: &super::steps::Step) -> Self {
        Self {
            environment: step.environment.clone(),
        }
    }
}

// the B8500 read a resolved reference gets: an os.environ/ name reads the map, and true/false read back as unset
impl SecretResolver for FixtureResolver {
    fn get_secret_str(&self, reference: &str) -> Option<SecretString> {
        let name = reference.replace("os.environ/", "");
        let text = self.environment.get(&name)?;
        let lowered = text.trim().to_lowercase();
        if lowered == "true" || lowered == "false" {
            return None;
        }
        Some(SecretString::from(text.clone()))
    }
}

pub(super) fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn input_of(step: &super::steps::Step) -> ClientSecretInput {
    ClientSecretInput {
        tenant_id: step.tenant_id.clone(),
        client_id: step.client_id.clone(),
        client_secret: SecretString::from(step.client_secret.clone()),
        scope: step.scope.clone(),
    }
}

pub(super) fn pairs(pairs: &[(String, String)]) -> Value {
    let map: BTreeMap<&str, &str> = pairs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    json!(map)
}

pub(super) fn request_json(request: &HttpRequest) -> Value {
    json!({
        "method": match request.method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
        },
        "url": request.url,
        "query": pairs(&request.query),
        "form": pairs(&request.form),
    })
}
