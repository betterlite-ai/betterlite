use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_arc_identity::{ArcTransport, MetadataRequest, MetadataResponse};
use betterlite_azure::azure_hosted_identity::request::{
    HostedRequest, HostedResponse, HostedTransport,
};
use betterlite_azure::azure_managed_identity::imds::{CONNECTION_TIMEOUT_SECONDS, ImdsTransport};
use betterlite_azure::oidc_exchange::transport::{HttpReply, Transport as OidcTransport};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

const KEPT_HEADERS: [&str; 6] = [
    "metadata",
    "secret",
    "x-identity-header",
    "x-client-sku",
    "x-client-ver",
    "x-ms-client-request-id",
];

#[derive(Clone, Deserialize)]
pub(crate) struct Reply {
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: String,
}

pub(crate) struct Shared {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Value>>,
    pub(super) sleeps: Mutex<Vec<f64>>,
}

impl Shared {
    pub(crate) fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
            sleeps: Mutex::new(Vec::new()),
        })
    }

    fn next(&self) -> io::Result<Reply> {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| io::Error::other("no reply queued"))
    }

    pub(crate) fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn sleeps(&self) -> Vec<f64> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn header_map(headers: &[(String, SecretString)]) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.expose_secret().to_owned()))
        .filter(|(name, _)| KEPT_HEADERS.contains(&name.as_str()))
        .collect()
}

fn record(shared: &Shared, request: Value) {
    shared
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request);
}

pub(crate) struct SharedTransport {
    pub(super) shared: Arc<Shared>,
}

impl ImdsTransport for SharedTransport {
    fn get(&self, request: &MetadataRequest) -> io::Result<MetadataResponse> {
        record(
            &self.shared,
            json!({
                "method": "GET",
                "url": request.url,
                "params": BTreeMap::from_iter(request.query.iter().cloned()),
                "headers": header_map(&request.headers),
                "form": null,
                // the Python pipeline only records a timeout a request sets itself, so the transport default stays null
                "timeout": (request.timeout.as_secs() != CONNECTION_TIMEOUT_SECONDS)
                    .then_some(request.timeout.as_secs()),
            }),
        );
        let reply = self.shared.next()?;
        Ok(MetadataResponse {
            status: reply.status,
            headers: reply.headers.into_iter().collect(),
            body: SecretString::from(reply.body),
        })
    }
}

impl OidcTransport for SharedTransport {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<HttpReply> {
        let recorded_form: BTreeMap<String, String> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        record(
            &self.shared,
            json!({
                "method": "POST",
                "url": url,
                "params": BTreeMap::<String, String>::new(),
                "headers": BTreeMap::<String, String>::new(),
                "form": recorded_form,
                "timeout": null,
            }),
        );
        let reply = self.shared.next()?;
        Ok(HttpReply {
            status: reply.status,
            body: SecretString::from(reply.body),
        })
    }
}

// the oracle cases reach hosted protocols through selection only, so a request here is a fixture bug
impl HostedTransport for SharedTransport {
    fn send(&self, _request: &HostedRequest) -> io::Result<HostedResponse> {
        Err(io::Error::other(
            "hosted identity is not part of the managed identity cases",
        ))
    }
}

impl ArcTransport for SharedTransport {
    fn get(&self, _request: &MetadataRequest) -> io::Result<MetadataResponse> {
        Err(io::Error::other(
            "Arc identity is not part of the managed identity cases",
        ))
    }
}
