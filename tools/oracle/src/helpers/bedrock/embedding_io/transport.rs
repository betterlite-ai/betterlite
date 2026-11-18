use std::collections::BTreeSet;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_bedrock::bedrock_embedding_io::transport::{
    ClientFactory, ClientKind, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, Timeout,
    TransferError,
};
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
    case.reply
        .headers
        .keys()
        .map(|name| name.to_ascii_lowercase())
        .collect()
}

pub(crate) fn case_timeout(case: &HttpRequest) -> Option<Timeout> {
    match (case.timeout, case.timeout_record) {
        (_, Some(record)) => Some(Timeout::Record {
            connect: record.connect,
            read: record.read,
            write: record.write,
            pool: record.pool,
        }),
        (Some(seconds), None) => Some(Timeout::Seconds(seconds)),
        (None, None) => None,
    }
}

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
                headers.insert(lowered, json!(value));
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
}

impl EmbeddingTransport for FixtureClient {
    fn post(
        &self,
        request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, TransferError>> + Send {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url,
                headers: request.headers,
                body: request.body,
            });
        let reply = match self.reply.failure {
            Some(ReplyFailure::Timeout) => Err(TransferError::Timeout),
            Some(ReplyFailure::Failed) => Err(TransferError::Failed(TransportError::Closed)),
            None => Ok(EmbeddingReply {
                status: self.reply.status,
                headers: super::signed::pairs(&self.reply.headers),
                body: self.reply.body.as_bytes().to_vec(),
            }),
        };
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
            "timeout": self.timeout.map(Timeout::fields),
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
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self, timeout: Option<Timeout>) -> Self::Client {
        self.add(FactoryCall {
            kind: "sync",
            provider: None,
            timeout,
        });
        FixtureClient {
            reply: self.reply.clone(),
            records: Arc::clone(&self.records),
        }
    }

    fn async_client(&self, provider: &str, timeout: Option<Timeout>) -> Self::Client {
        self.add(FactoryCall {
            kind: "async",
            provider: Some(provider.to_owned()),
            timeout,
        });
        FixtureClient {
            reply: self.reply.clone(),
            records: Arc::clone(&self.records),
        }
    }
}

pub(crate) fn supplied_client(case: &HttpRequest) -> Option<(ClientKind, FixtureClient)> {
    let kind = match case.supplied {
        Supplied::None => return None,
        Supplied::Sync => ClientKind::Sync,
        Supplied::Async => ClientKind::Async,
    };
    Some((kind, FixtureClient::new(case.reply.clone())))
}
