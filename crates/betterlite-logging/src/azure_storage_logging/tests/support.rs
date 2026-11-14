#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;

use crate::azure_storage_logging::filename::IdSource;
use crate::azure_storage_logging::logger::{AzureBlobStorageLogger, AzureStorageDeps};
use crate::azure_storage_logging::settings::AzureStorageSettings;
use crate::azure_storage_logging::token::{ChainError, ChainProviderFactory, ChainTokenProvider};
use crate::azure_storage_logging::transport::{
    StorageRequest, StorageResponse, StorageTransport, StorageTransportError,
};
use crate::callback_logger::event::LoggerEvent;
use crate::datadog_logging::clock::Clock;
use crate::fields::payload::StandardLoggingPayload;

pub(crate) const PAYLOAD: &str = include_str!("payload.json");
pub(crate) const START_SECONDS: u64 = 1_700_000_000;

pub(crate) fn payload(id: &str) -> StandardLoggingPayload {
    let text = PAYLOAD.replace("\"id-1\"", &format!("\"{id}\""));
    serde_json::from_str(&text).expect("the payload fixture matches StandardLoggingPayload")
}

pub(crate) fn event(id: &str) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload(id)),
        dynamic_params: Arc::new(Json::Null),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(crate) struct ManualClock(pub(crate) Mutex<Duration>);

impl ManualClock {
    pub(crate) fn new(seconds: u64) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Duration::from_secs(seconds))))
    }

    pub(crate) fn advance(&self, seconds: u64) {
        let mut now = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(Duration::from_secs(seconds));
    }
}

impl Clock for ManualClock {
    fn now_unix(&self) -> Duration {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: &'static str,
}

impl Reply {
    pub(crate) const OK: Self = Self {
        status: 200,
        body: "",
    };
    pub(crate) const NOT_FOUND: Self = Self {
        status: 404,
        body: "",
    };
    pub(crate) const TOKEN: Self = Self {
        status: 200,
        body: r#"{"access_token": "tok-1"}"#,
    };
}

#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Call {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(present, _value)| present.eq_ignore_ascii_case(name))
            .map(|(_name, value)| value.as_str())
    }
}

pub(crate) struct ScriptedTransport {
    pub(crate) replies: Mutex<VecDeque<Reply>>,
    pub(crate) calls: Mutex<Vec<Call>>,
}

impl ScriptedTransport {
    pub(crate) fn new(replies: &[Reply]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().copied().collect()),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl StorageTransport for ScriptedTransport {
    async fn send(
        &self,
        request: StorageRequest,
    ) -> Result<StorageResponse, StorageTransportError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                method: request.method.as_str().to_owned(),
                url: request.url,
                headers: request.headers,
                body: request.body,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Reply::OK);
        Ok(StorageResponse {
            status: reply.status,
            body: reply.body.as_bytes().to_vec(),
        })
    }
}

pub(crate) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        "generated-id".to_owned()
    }
}

pub(crate) struct FixedChain {
    pub(crate) token: SecretString,
    pub(crate) calls: AtomicUsize,
}

impl ChainTokenProvider for FixedChain {
    fn token(&self) -> Result<SecretString, ChainError> {
        let _previous = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.token.clone())
    }
}

pub(crate) struct FixedChainFactory {
    pub(crate) provider: Arc<FixedChain>,
    pub(crate) built: AtomicUsize,
}

impl ChainProviderFactory for FixedChainFactory {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError> {
        let _previous = self.built.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::<FixedChain>::clone(&self.provider))
    }
}

pub(crate) struct Fixture {
    pub(crate) settings: AzureStorageSettings,
    pub(crate) transport: Arc<ScriptedTransport>,
    pub(crate) clock: Arc<ManualClock>,
    pub(crate) chain: Arc<FixedChain>,
    pub(crate) factory: Arc<FixedChainFactory>,
    pub(crate) logger: Arc<AzureBlobStorageLogger>,
}

pub(crate) fn environment(variables: &[(&str, &str)]) -> BTreeEnvironment {
    BTreeEnvironment(
        variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    )
}

pub(crate) struct BTreeEnvironment(pub(crate) BTreeMap<String, String>);

impl EnvironmentLookup for BTreeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

pub(crate) fn fixture(variables: &[(&str, &str)], replies: &[Reply]) -> Fixture {
    let environment = environment(variables);
    let settings = AzureStorageSettings::from_environment(&environment)
        .expect("the fixture environment names the account and file system");
    let transport = ScriptedTransport::new(replies);
    let clock = ManualClock::new(START_SECONDS);
    let chain = Arc::new(FixedChain {
        token: SecretString::from("chain-token"),
        calls: AtomicUsize::new(0),
    });
    let factory = Arc::new(FixedChainFactory {
        provider: Arc::clone(&chain),
        built: AtomicUsize::new(0),
    });
    let logger = AzureBlobStorageLogger::new(
        settings.clone(),
        AzureStorageDeps {
            transport: Arc::<ScriptedTransport>::clone(&transport),
            clock: Arc::<ManualClock>::clone(&clock),
            environment: Arc::new(environment),
            ids: Arc::new(FixedIds),
            chain_factory: Arc::<FixedChainFactory>::clone(&factory),
            shutdown: CancellationToken::new(),
            tasks: None,
        },
    );
    Fixture {
        settings,
        transport,
        clock,
        chain,
        factory,
        logger,
    }
}

pub(crate) const ACCOUNT: [(&str, &str); 3] = [
    ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
    ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
    ("AZURE_STORAGE_ACCOUNT_KEY", "a2V5"),
];

pub(crate) const ENTRA: [(&str, &str); 5] = [
    ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
    ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
    ("AZURE_STORAGE_TENANT_ID", "tenant-1"),
    ("AZURE_STORAGE_CLIENT_ID", "client-1"),
    ("AZURE_STORAGE_CLIENT_SECRET", "secret-1"),
];
