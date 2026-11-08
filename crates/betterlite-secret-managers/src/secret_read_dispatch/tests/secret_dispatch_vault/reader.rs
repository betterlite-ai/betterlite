use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::{ExposeSecret, SecretString};
use tokio::time::Instant;

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
fn vault_reader_returns_none_on_an_error_status() {
    let transport = Arc::new(ScriptedVault::new(404, "{\"errors\":[]}"));
    let clock: Clock = Arc::new(Instant::now);
    let reader = VaultManagerReader::new(&environment(), transport, clock).expect("a reader");
    assert!(reader.sync_read_secret("db").is_none());
}
