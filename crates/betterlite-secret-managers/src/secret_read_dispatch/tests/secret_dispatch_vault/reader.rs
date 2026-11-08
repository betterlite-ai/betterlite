use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::{ExposeSecret, SecretString};
use tokio::time::Instant;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::fmt;

use crate::cyberark::Clock;
use crate::secret_read_dispatch::manager_readers::VaultManagerReader;
use crate::secret_read_dispatch::protocols::SyncSecretReader as _;
use crate::vault::transport::{VaultRequest, VaultResponse, VaultTransport};

const KV_BODY: &str = r#"{"data":{"data":{"key":"vault-value"}}}"#;

struct ScriptedVault {
    status: u16,
    body: String,
    requests: Mutex<Vec<VaultRequest>>,
}

impl ScriptedVault {
    fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn answer(&self, request: VaultRequest) -> VaultResponse {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
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

#[test]
fn vault_reader_reads_through_the_manager() {
    let transport = Arc::new(ScriptedVault::new(200, KV_BODY));
    let clock: Clock = Arc::new(Instant::now);
    let reader =
        VaultManagerReader::new(&environment(), Arc::clone(&transport), clock).expect("a reader");
    let secret = reader.sync_read_secret("db").expect("a secret");
    assert_eq!(secret.expose_secret(), "vault-value");
    let requests = transport
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "http://127.0.0.1:8200/v1/secret/data/db");
    assert_eq!(
        requests[0].token.as_ref().map(ExposeSecret::expose_secret),
        Some("token")
    );
}

#[test]
fn vault_reader_leaves_the_namespace_out_of_the_header() {
    let transport = Arc::new(ScriptedVault::new(200, KV_BODY));
    let clock: Clock = Arc::new(Instant::now);
    let mut vars = environment();
    vars.insert("HCP_VAULT_NAMESPACE".to_owned(), "team".to_owned());
    let reader = VaultManagerReader::new(&vars, Arc::clone(&transport), clock).expect("a reader");
    assert!(reader.sync_read_secret("db").is_some());
    let requests = transport
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        requests[0].url,
        "http://127.0.0.1:8200/v1/team/secret/data/db"
    );
    assert!(requests[0].namespace.is_none());
    drop(requests);
}

#[test]
fn vault_reader_returns_none_on_an_error_status_and_logs_it() {
    let (logs, _guard) = capture_logs();
    let transport = Arc::new(ScriptedVault::new(404, "{\"errors\":[]}"));
    let clock: Clock = Arc::new(Instant::now);
    let reader = VaultManagerReader::new(&environment(), transport, clock).expect("a reader");
    assert!(reader.sync_read_secret("db").is_none());
    assert!(
        logs.captured()
            .contains("Error reading secret from Hashicorp Vault: Vault answered with status 404")
    );
}

#[test]
fn vault_reader_logs_a_failed_transport_read() {
    let (logs, _guard) = capture_logs();
    let clock: Clock = Arc::new(Instant::now);
    let reader =
        VaultManagerReader::new(&environment(), Arc::new(FailingVault), clock).expect("a reader");
    assert!(reader.sync_read_secret("db").is_none());
    assert!(
        logs.captured()
            .contains("Error reading secret from Hashicorp Vault: connection refused")
    );
}
