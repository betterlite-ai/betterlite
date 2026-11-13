use std::collections::{BTreeMap, VecDeque};
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
    token: Option<String>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: &[Reply], token: Option<String>) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            calls: Mutex::new(Vec::new()),
            token,
        }
    }

    pub(super) fn described(&self, account_key: bool) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|call| describe(call, account_key))
            .collect()
    }
}

fn header(call: &StorageRequest, name: &str) -> Option<String> {
    call.headers
        .iter()
        .find(|(present, _value)| present.eq_ignore_ascii_case(name))
        .map(|(_name, value)| value.clone())
}

fn describe(call: &StorageRequest, account_key: bool) -> Value {
    let url = call.url.clone();
    if url.ends_with(TOKEN_SUFFIX) {
        let form: BTreeMap<String, String> = url::form_urlencoded::parse(&call.body)
            .into_owned()
            .collect();
        let tenant = url::Url::parse(&url).ok().and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|mut segments| segments.next().map(str::to_owned))
        });
        return json!({
            "op": "entra_token",
            "tenant_id": tenant,
            "client_id": form.get("client_id"),
            "client_secret": form.get("client_secret"),
            "scope": form.get("scope"),
        });
    }
    let mut described = json!({ "op": operation(call, &url), "url": url });
    if account_key {
        return described;
    }
    if let Some(fields) = described.as_object_mut() {
        fields.insert(
            "x_ms_version".to_owned(),
            json!(header(call, "x-ms-version")),
        );
        fields.insert(
            "content_type".to_owned(),
            json!(header(call, "Content-Type")),
        );
        fields.insert(
            "content_length".to_owned(),
            json!(header(call, "Content-Length")),
        );
        let scheme = header(call, "Authorization")
            .and_then(|value| value.split_whitespace().next().map(str::to_owned));
        fields.insert("authorization_scheme".to_owned(), json!(scheme));
    }
    described
}

fn operation(call: &StorageRequest, url: &str) -> &'static str {
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
            let token = self.token.clone().unwrap_or_else(|| "tok-1".to_owned());
            return Ok(StorageResponse {
                status: 200,
                body: format!(r#"{{"access_token": "{token}"}}"#).into_bytes(),
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
