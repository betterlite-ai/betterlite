use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::azure_storage_logging::filename::IdSource;
use betterlite_logging::azure_storage_logging::token::{
    ChainError, ChainProviderFactory, ChainTokenProvider,
};
use betterlite_logging::azure_storage_logging::transport::{
    StorageRequest, StorageResponse, StorageTransport, StorageTransportError,
};
use betterlite_logging::datadog_logging::clock::Clock;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{CHAIN_TOKEN, Reply, TOKEN_SUFFIX};

pub(super) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        "generated-id".to_owned()
    }
}

#[derive(Default)]
pub(super) struct FixedChain {
    pub(super) calls: Mutex<usize>,
}

impl ChainTokenProvider for FixedChain {
    fn token(&self) -> Result<SecretString, ChainError> {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        Ok(SecretString::from(CHAIN_TOKEN))
    }
}

pub(super) struct FixedFactory {
    pub(super) provider: Arc<FixedChain>,
    pub(super) built: Mutex<usize>,
}

impl ChainProviderFactory for FixedFactory {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError> {
        *self.built.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        Ok(self.provider.clone())
    }
}

pub(super) struct ManualClock(pub(super) Mutex<Duration>);

impl Clock for ManualClock {
    fn now_unix(&self) -> Duration {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<StorageRequest>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn described(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(describe)
            .collect()
    }
}

fn operation(call: &StorageRequest) -> &'static str {
    let url = call.url.as_str();
    if url.ends_with(TOKEN_SUFFIX) {
        return "entra_token";
    }
    if call.method == http::Method::HEAD {
        return "exists";
    }
    if url.contains("resource=directory") {
        return "create_directory";
    }
    if url.contains("resource=file") {
        return "create_file";
    }
    if url.contains("action=append") {
        return "append";
    }
    if url.contains("action=flush") {
        return "flush";
    }
    "other"
}

fn describe(call: &StorageRequest) -> Value {
    let op = operation(call);
    if op == "entra_token" {
        return json!({ "op": op });
    }
    let mut described = json!({ "op": op, "url": call.url });
    if op == "append" {
        let text = std::str::from_utf8(&call.body).unwrap_or_default();
        let body: Option<Value> = serde_json::from_str(text.trim_end_matches('\n')).ok();
        if let Some(fields) = described.as_object_mut() {
            fields.insert("body".to_owned(), json!(body));
            fields.insert("trailing_newline".to_owned(), json!(text.ends_with('\n')));
        }
    }
    described
}

#[async_trait]
impl StorageTransport for ScriptedTransport {
    async fn send(
        &self,
        request: StorageRequest,
    ) -> Result<StorageResponse, StorageTransportError> {
        let token_request = request.url.ends_with(TOKEN_SUFFIX);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if token_request {
            return Ok(StorageResponse {
                status: 200,
                body: br#"{"access_token": "tok-1"}"#.to_vec(),
            });
        }
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default();
        Ok(StorageResponse {
            status: reply.status,
            body: reply.body.into_bytes(),
        })
    }
}
