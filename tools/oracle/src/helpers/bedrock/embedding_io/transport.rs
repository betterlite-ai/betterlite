use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, ReplyRequest, Timeout,
    TimeoutRecord, TransferError,
};
use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};

use super::failure::Failure;
use super::input::{Failure as ReplyFailure, HttpRequest, Reply, Supplied};

pub(crate) fn case_headers(case: &HttpRequest) -> BTreeSet<String> {
    case.headers
        .keys()
        .map(|name| name.to_ascii_lowercase())
        .collect()
}

pub(crate) fn reply_headers(case: &HttpRequest) -> BTreeSet<String> {
    let names = |headers: &BTreeMap<String, String>| {
        headers
            .keys()
            .map(|name| name.to_ascii_lowercase())
            .collect::<BTreeSet<String>>()
    };
    let mut all = names(&case.reply.headers);
    if let Some(redirect) = &case.reply.redirect {
        all.extend(names(&redirect.headers));
    }
    all
}

pub(crate) fn case_timeout(case: &HttpRequest) -> Option<Timeout> {
    match (case.timeout, case.timeout_record) {
        (_, Some(record)) => Some(Timeout::Record(Box::new(TimeoutRecord {
            connect: record.connect,
            read: record.read,
            write: record.write,
            pool: record.pool,
        }))),
        (Some(seconds), None) => Some(Timeout::Seconds(seconds)),
        (None, None) => None,
    }
}

#[derive(Clone)]
pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

pub(crate) fn request_values(
    records: &[Recorded],
    names: &BTreeSet<String>,
) -> Result<Vec<Value>, Failure> {
    let mut values = Vec::with_capacity(records.len());
    for record in records {
        let mut headers = Object::new();
        for (name, value) in &record.headers {
            let lowered = name.to_ascii_lowercase();
            if names.contains(&lowered) {
                headers.insert(lowered, serde_json::from_value(json!(value))?);
            }
        }
        values.push(json!({
            "url": record.url,
            "headers": headers,
            "data": serde_json::from_slice::<Value>(&record.body)?,
        }));
    }
    Ok(values)
}

pub(crate) struct FixtureClient {
    reply: Reply,
    records: Arc<Mutex<Vec<Recorded>>>,
}

impl FixtureClient {
    pub(crate) fn new(reply: Reply) -> Self {
        Self {
            reply,
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn recorded(&self) -> Vec<Recorded> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // a followed redirect answers with the final request the transport actually sent
    fn answer(&self, request: &EmbeddingRequest) -> EmbeddingReply {
        if let Some(redirect) = &self.reply.redirect {
            return EmbeddingReply {
                status: redirect.status,
                headers: super::signed::pairs(&redirect.headers),
                body: redirect.body.as_bytes().to_vec(),
                request: ReplyRequest {
                    method: redirect.method.clone(),
                    url: redirect.url.clone(),
                },
            };
        }
        EmbeddingReply {
            status: self.reply.status,
            headers: super::signed::pairs(&self.reply.headers),
            body: self.reply.body.as_bytes().to_vec(),
            request: ReplyRequest {
                method: "POST".to_owned(),
                url: request.url.clone(),
            },
        }
    }
}

impl EmbeddingTransport for FixtureClient {
    fn post(
        &self,
        request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, TransferError>> + Send {
        let reply = match self.reply.failure {
            Some(ReplyFailure::Timeout) => Err(TransferError::Timeout {
                elapsed: Duration::from_millis(125),
            }),
            Some(ReplyFailure::Failed) => Err(TransferError::Failed(TransportError::Closed)),
            None => Ok(self.answer(&request)),
        };
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url,
                headers: request.headers,
                body: request.body,
            });
        ready(reply)
    }
}

pub(crate) struct FactoryCall {
    kind: &'static str,
    provider: Option<String>,
    timeout: Option<Timeout>,
}

impl FactoryCall {
    fn value(&self) -> Value {
        json!({
            "kind": self.kind,
            "provider": self.provider,
            "timeout": self.timeout.as_ref().map(Timeout::fields),
        })
    }
}

pub(crate) struct Factory {
    reply: Reply,
    records: Arc<Mutex<Vec<Recorded>>>,
    calls: Mutex<Vec<FactoryCall>>,
}

impl Factory {
    pub(crate) fn new(reply: Reply) -> Self {
        Self {
            reply,
            records: Arc::new(Mutex::new(Vec::new())),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(FactoryCall::value)
            .collect()
    }

    pub(crate) fn recorded(&self) -> Vec<Recorded> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn add(&self, call: FactoryCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn client(&self) -> FixtureClient {
        FixtureClient {
            reply: self.reply.clone(),
            records: Arc::clone(&self.records),
        }
    }
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self, timeout: Option<Timeout>) -> Self::Client {
        self.add(FactoryCall {
            kind: "sync",
            provider: None,
            timeout,
        });
        self.client()
    }

    fn async_client(&self, provider: &str, timeout: Option<Timeout>) -> Self::Client {
        self.add(FactoryCall {
            kind: "async",
            provider: Some(provider.to_owned()),
            timeout,
        });
        self.client()
    }
}

pub(crate) fn supplied_client(case: &HttpRequest) -> Option<(ClientKind, FixtureClient)> {
    let kind = match case.supplied {
        Supplied::None => return None,
        Supplied::Sync => ClientKind::Sync,
        Supplied::Async => ClientKind::Asynchronous,
    };
    Some((kind, FixtureClient::new(case.reply.clone())))
}
