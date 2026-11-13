use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use secrecy::{ExposeSecret, SecretString};
use tokio::time::Instant;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::fmt;

use crate::cyberark::Clock;
use crate::secret_read_dispatch::protocols::SyncSecretReader as _;
use crate::vault::HashicorpSecretManager;
use crate::vault::transport::{VaultRequest, VaultResponse, VaultTransport};

const KV_BODY: &str = r#"{"data":{"data":{"key":"vault-value"}}}"#;

#[derive(Clone)]
struct ScriptedVault {
    status: u16,
    body: String,
    requests: Arc<Mutex<Vec<VaultRequest>>>,
}

impl ScriptedVault {
    fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn recorded(&self) -> MutexGuard<'_, Vec<VaultRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn request_count(&self) -> usize {
        self.recorded().len()
    }

    fn answer(&self, request: VaultRequest) -> VaultResponse {
        self.recorded().push(request);
        VaultResponse {
            status: self.status,
            body: SecretString::from(self.body.as_str()),
        }
    }
}

impl VaultTransport for ScriptedVault {
    type Error = IoError;

    fn send(
        &self,
        request: VaultRequest,
    ) -> impl Future<Output = Result<VaultResponse, IoError>> + Send {
        ready(Ok(self.answer(request)))
    }

    fn send_blocking(&self, request: VaultRequest) -> Result<VaultResponse, IoError> {
        Ok(self.answer(request))
    }
}

struct FailingVault;

impl VaultTransport for FailingVault {
    type Error = IoError;

    fn send(
        &self,
        _request: VaultRequest,
    ) -> impl Future<Output = Result<VaultResponse, IoError>> + Send {
        ready(Err(IoError::other("connection refused")))
    }

    fn send_blocking(&self, _request: VaultRequest) -> Result<VaultResponse, IoError> {
        Err(IoError::other("connection refused"))
    }
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn captured(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("utf-8 logs")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let guard = set_default(fmt().with_ansi(false).with_writer(buffer.clone()).finish());
    (buffer, guard)
}

fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("HCP_VAULT_TOKEN".to_owned(), "token".to_owned()),
        (
            "HCP_VAULT_ADDR".to_owned(),
            "http://127.0.0.1:8200".to_owned(),
        ),
    ])
}

fn manager<T>(environment: &BTreeMap<String, String>, transport: T) -> HashicorpSecretManager<T>
where
    T: VaultTransport,
{
    let clock: Clock = Arc::new(Instant::now);
    HashicorpSecretManager::new(environment, transport, clock).expect("a manager")
}

#[test]
fn vault_reader_reads_through_the_manager() {
    let vault = ScriptedVault::new(200, KV_BODY);
    let reader = manager(&environment(), vault.clone());
    let secret = reader.sync_read_secret("db").expect("a secret");
    assert_eq!(secret.expose_secret(), "vault-value");
    let requests = vault.recorded();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "http://127.0.0.1:8200/v1/secret/data/db");
    assert!(requests[0].namespace.is_none());
    assert_eq!(
        requests[0].token.as_ref().map(ExposeSecret::expose_secret),
        Some("token")
    );
    drop(requests);
}

#[test]
fn vault_reader_leaves_the_namespace_out_of_the_header() {
    let vault = ScriptedVault::new(200, KV_BODY);
    let mut vars = environment();
    vars.insert("HCP_VAULT_NAMESPACE".to_owned(), "team".to_owned());
    let reader = manager(&vars, vault.clone());
    assert!(reader.sync_read_secret("db").is_some());
    let requests = vault.recorded();
    assert_eq!(
        requests[0].url,
        "http://127.0.0.1:8200/v1/team/secret/data/db"
    );
    assert!(requests[0].namespace.is_none());
    drop(requests);
}

#[test]
fn vault_reader_serves_the_second_read_from_the_body_cache() {
    let vault = ScriptedVault::new(200, KV_BODY);
    let reader = manager(&environment(), vault.clone());
    assert!(reader.sync_read_secret("db").is_some());
    assert!(reader.sync_read_secret("db").is_some());
    assert_eq!(vault.request_count(), 1);
}

#[test]
fn vault_reader_returns_none_on_an_error_status_and_logs_it() {
    let (logs, _guard) = capture_logs();
    let reader = manager(&environment(), ScriptedVault::new(404, "{\"errors\":[]}"));
    assert!(reader.sync_read_secret("db").is_none());
    assert!(
        logs.captured()
            .contains("error reading secret from Hashicorp Vault: Vault answered with status 404")
    );
}

#[test]
fn vault_reader_logs_a_non_json_body() {
    let (logs, _guard) = capture_logs();
    let reader = manager(&environment(), ScriptedVault::new(200, "not json"));
    assert!(reader.sync_read_secret("db").is_none());
    assert!(logs.captured().contains(
        "error reading secret from Hashicorp Vault: the Vault response body is not a JSON object"
    ));
}

#[test]
fn vault_reader_logs_a_failed_transport_read() {
    let (logs, _guard) = capture_logs();
    let reader = manager(&environment(), FailingVault);
    assert!(reader.sync_read_secret("db").is_none());
    assert!(
        logs.captured().contains(
            "error reading secret from Hashicorp Vault: the Vault request did not complete"
        )
    );
}
