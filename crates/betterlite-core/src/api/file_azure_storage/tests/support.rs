#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::api::file_azure_storage::Settings;
use betterlite_logging::azure_storage_logging::filename::IdSource;
use betterlite_logging::azure_storage_logging::token::{
    ChainError, ChainProviderFactory, ChainTokenProvider,
};
use betterlite_logging::azure_storage_logging::transport::{
    StorageRequest, StorageResponse, StorageTransport, StorageTransportError,
};
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::{AzureBlobStorageBackend, AzureFileStorageDeps};

pub(crate) const NOW_MILLIS: u64 = 1_234;
pub(crate) const FIXED_ID: &str = "00000000-0000-0000-0000-000000000001";
const PAYLOAD: &str = include_str!("payload.json");

pub(crate) fn settings() -> Settings {
    Settings {
        account_name: "acct".to_owned(),
        file_system: "files".to_owned(),
        endpoint_suffix: "core.windows.net".to_owned(),
        dfs_endpoint: "https://acct.dfs.core.windows.net".to_owned(),
        blob_endpoint: "https://acct.blob.core.windows.net".to_owned(),
        tenant_id: None,
        client_id: None,
        client_secret: None,
        account_key: None,
    }
}

pub(crate) fn settings_with_account_key() -> Settings {
    let mut settings = settings();
    settings.account_key = Some(SecretString::from("a2V5"));
    settings
}

pub(crate) fn event() -> Arc<LoggerEvent> {
    let payload: StandardLoggingPayload =
        serde_json::from_str(PAYLOAD).expect("the payload fixture matches StandardLoggingPayload");
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Null),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(crate) struct ManualClock(Mutex<Duration>);

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Duration::from_millis(NOW_MILLIS))))
    }

    pub(crate) fn advance(&self, duration: Duration) {
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(duration);
    }
}

impl Clock for ManualClock {
    fn now_unix(&self) -> Duration {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) struct FixedIds(String);

impl FixedIds {
    pub(crate) fn new(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        self.0.clone()
    }
}

struct FixedToken;

impl ChainTokenProvider for FixedToken {
    fn token(&self) -> Result<SecretString, ChainError> {
        Ok(SecretString::from("tok-1"))
    }
}

struct FixedChain;

impl ChainProviderFactory for FixedChain {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError> {
        Ok(Arc::new(FixedToken))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Recorded {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Recorded {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _value)| key.eq_ignore_ascii_case(name))
            .map(|(_key, value)| value.as_str())
    }
}

struct Recorder {
    replies: Mutex<VecDeque<StorageResponse>>,
    recorded: Mutex<Vec<Recorded>>,
}

impl Recorder {
    fn new(replies: &[(u16, &str)]) -> Arc<Self> {
        let replies = replies
            .iter()
            .map(|(status, body)| StorageResponse {
                status: *status,
                body: body.as_bytes().to_vec(),
            })
            .collect();
        Arc::new(Self {
            replies: Mutex::new(replies),
            recorded: Mutex::new(Vec::new()),
        })
    }

    fn recorded(&self) -> Vec<Recorded> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl StorageTransport for Recorder {
    async fn send(
        &self,
        request: StorageRequest,
    ) -> Result<StorageResponse, StorageTransportError> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                method: request.method.to_string(),
                url: request.url,
                headers: request.headers,
                body: request.body,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| StorageResponse {
                status: 200,
                body: Vec::new(),
            });
        Ok(reply)
    }
}

pub(crate) struct Harness {
    pub(crate) backend: Arc<AzureBlobStorageBackend>,
    pub(crate) clock: Arc<ManualClock>,
    recorder: Arc<Recorder>,
}

pub(crate) fn paths(calls: &[Recorded]) -> Vec<(String, String)> {
    calls
        .iter()
        .map(|call| (call.method.clone(), call.url.clone()))
        .collect()
}

impl Harness {
    pub(crate) fn new(settings: Settings, replies: &[(u16, &str)]) -> Self {
        let recorder = Recorder::new(replies);
        let clock = ManualClock::new();
        let backend = AzureBlobStorageBackend::new(
            settings,
            AzureFileStorageDeps {
                transport: Arc::clone(&recorder),
                clock: Arc::clone(&clock),
                environment: Arc::new(BTreeMap::new()),
                ids: Arc::new(FixedIds(FIXED_ID.to_owned())),
                chain_factory: Arc::new(FixedChain),
            },
        );
        Self {
            backend,
            clock,
            recorder,
        }
    }

    pub(crate) fn calls(&self) -> Vec<Recorded> {
        self.recorder.recorded()
    }
}
