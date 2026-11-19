#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::io;
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
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::api::file_azure_storage::AzureFileStorageDeps;
use crate::api::file_storage_service::factory::StorageBackendDeps;
use crate::api::file_storage_service::service::StorageUploadDeps;

pub(crate) const FIXED_ID: &str = "12345678-1234-5678-1234-567812345678";
const NOW_SECONDS: f64 = 1234.9;

pub(crate) struct FixedClock;

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        Duration::try_from_secs_f64(NOW_SECONDS).expect("the fixed clock is a finite duration")
    }
}

pub(crate) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        FIXED_ID.to_owned()
    }
}

pub(crate) struct HexIds;

impl IdSource for HexIds {
    fn next_id(&self) -> String {
        "abcdef12-3456-7890-abcd-ef1234567890".to_owned()
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

pub(crate) struct Recorder {
    replies: Mutex<VecDeque<StorageResponse>>,
    recorded: Mutex<Vec<Recorded>>,
}

impl Recorder {
    pub(crate) fn new(replies: &[(u16, &str)]) -> Arc<Self> {
        let scripted = replies
            .iter()
            .map(|(status, body)| StorageResponse {
                status: *status,
                body: body.as_bytes().to_vec(),
            })
            .collect();
        Arc::new(Self {
            replies: Mutex::new(scripted),
            recorded: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<Recorded> {
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
            // an unscripted request fails, so a stray call cannot look like a successful upload
            .unwrap_or(StorageResponse {
                status: 500,
                body: Vec::new(),
            });
        Ok(reply)
    }
}

// the test asserts a debug line by reading every record the thread writes
#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log is text")
    }
}

impl io::Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(captured.clone())
        .finish();
    (captured, set_default(subscriber))
}

pub(crate) fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("AZURE_STORAGE_ACCOUNT_NAME".to_owned(), "acct".to_owned()),
        ("AZURE_STORAGE_FILE_SYSTEM".to_owned(), "files".to_owned()),
    ])
}

pub(crate) fn backend_deps<R>(
    recorder: &Arc<Recorder>,
    database: Option<R>,
    environment: BTreeMap<String, String>,
) -> StorageBackendDeps<R> {
    let transport: Arc<dyn StorageTransport> = Arc::clone(recorder);
    StorageBackendDeps {
        azure: AzureFileStorageDeps {
            transport,
            clock: Arc::new(FixedClock),
            environment: Arc::new(environment),
            ids: Arc::new(FixedIds),
            chain_factory: Arc::new(FixedChain),
        },
        azure_settings: None,
        database,
    }
}

pub(crate) fn upload_deps<R>(backend: StorageBackendDeps<R>) -> StorageUploadDeps<'static, R> {
    StorageUploadDeps {
        backend,
        clock: &FixedClock,
        ids: &FixedIds,
    }
}
