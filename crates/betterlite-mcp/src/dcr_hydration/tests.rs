#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPTokenEndpointAuthMethod};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::{
    DcrClientStore, DcrHydration, PersistedDcrCredentials, StoreReadError,
    redirect_uri_not_registered,
};
use crate::server_record::{MCPServer, SecretCredential};

const SALT_KEY: &str = "b6595-salt-key";

#[derive(Debug, Error)]
#[error("the store is unreachable")]
struct StoreDown;

struct FakeStore {
    blob: Result<Option<JsonValue>, StoreDown>,
    reads: AtomicUsize,
    last_server: Mutex<Option<String>>,
}

impl FakeStore {
    fn returning(blob: Option<JsonValue>) -> Self {
        Self {
            blob: Ok(blob),
            reads: AtomicUsize::new(0),
            last_server: Mutex::new(None),
        }
    }

    fn failing() -> Self {
        Self {
            blob: Err(StoreDown),
            reads: AtomicUsize::new(0),
            last_server: Mutex::new(None),
        }
    }
}

impl DcrClientStore for FakeStore {
    type Error = StoreDown;

    fn client_credentials(
        &self,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonValue>, StoreReadError<Self::Error>>> + Send {
        self.reads.fetch_add(1, Ordering::SeqCst);
        *self.last_server.lock().unwrap() = Some(server_id.to_owned());
        let result = match &self.blob {
            Ok(blob) => Ok(blob.clone()),
            Err(_) => Err(StoreReadError::Read { source: StoreDown }),
        };
        ready(result)
    }
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    fn messages(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Current> Layer<Current> for Logs
where
    Current: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Current>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0.lock().unwrap().push(message.0);
    }
}

fn cipher_with(key: Option<&str>) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: key.map(|text| SecretString::from(text.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn sealed(cipher: &DbCipher, text: &str) -> String {
    let sealed = cipher
        .encrypt_value(
            StoredValue::<Infallible>::Text(SecretString::from(text.to_owned())),
            None,
        )
        .unwrap();
    match sealed {
        StoredValue::Text(secret) => secret.expose_secret().to_owned(),
        StoredValue::Other(never) => match never {},
    }
}

fn blob(fields: &[(&str, JsonValue)]) -> JsonValue {
    JsonValue::Object(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

fn server() -> MCPServer {
    serde_json::from_str(r#"{"server_id": "srv", "name": "Server", "transport": "http"}"#).unwrap()
}

fn server_with_client_id(client_id: &str) -> MCPServer {
    let mut server = server();
    server.fields.client_id = Some(client_id.to_owned());
    server
}

fn with_client_secret(mut server: MCPServer, secret: &str) -> MCPServer {
    server.fields.client_secret = Some(SecretCredential(StoredSecret(SecretString::from(
        secret.to_owned(),
    ))));
    server
}

fn client_secret(server: &MCPServer) -> Option<String> {
    server
        .fields
        .client_secret
        .as_ref()
        .map(|credential| credential.0.0.expose_secret().to_owned())
}

fn hydration(store: FakeStore, cipher: DbCipher) -> DcrHydration<FakeStore> {
    DcrHydration { store, cipher }
}

fn credentials(redirect_uris: Option<Vec<String>>) -> PersistedDcrCredentials {
    PersistedDcrCredentials {
        client_id: None,
        client_secret: None,
        token_endpoint_auth_method: None,
        redirect_uris,
    }
}

#[derive(Clone, Copy)]
enum BlobShape {
    Object,
    JsonString,
}

#[tokio::test]
async fn a_configured_client_id_wins_and_the_store_is_not_read() {
    let stored = blob(&[("client_id", JsonValue::String("stored-client".to_owned()))]);
    let hydration = hydration(
        FakeStore::returning(Some(stored)),
        cipher_with(Some(SALT_KEY)),
    );
    let mut server = server_with_client_id("configured-client");
    assert!(
        !hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(
        server.fields.client_id.as_deref(),
        Some("configured-client")
    );
    assert_eq!(hydration.store.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_store_without_a_row_leaves_the_server_alone() {
    let hydration = hydration(FakeStore::returning(None), cipher_with(Some(SALT_KEY)));
    let mut server = server();
    assert!(
        !hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(hydration.store.reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        hydration.store.last_server.lock().unwrap().as_deref(),
        Some("srv")
    );
    assert_eq!(server.fields.client_id, None);
    assert_eq!(server.fields.client_secret, None);
}

#[tokio::test]
async fn a_store_read_error_is_logged_at_debug_and_leaves_the_server_alone() {
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let hydration = hydration(FakeStore::failing(), cipher_with(Some(SALT_KEY)));
    let mut server = with_client_secret(server(), "leftover-secret");
    assert!(
        !hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(server.fields.client_id, None);
    assert_eq!(client_secret(&server).as_deref(), Some("leftover-secret"));
    let messages = logs.messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with(
        "register_client_with_server: failed to read stored DCR client for server_id=srv: "
    ));
    assert!(messages[0].contains("the store is unreachable"));
}

#[rstest]
#[case::object(BlobShape::Object)]
#[case::json_string(BlobShape::JsonString)]
#[tokio::test]
async fn hydrate_overlays_the_stored_client(#[case] shape: BlobShape) {
    let cipher = cipher_with(Some(SALT_KEY));
    let object = blob(&[
        (
            "client_id",
            JsonValue::String(sealed(&cipher, "stored-client")),
        ),
        (
            "client_secret",
            JsonValue::String(sealed(&cipher, "stored-secret")),
        ),
        (
            "token_endpoint_auth_method",
            JsonValue::String("client_secret_post".to_owned()),
        ),
    ]);
    let stored = match shape {
        BlobShape::Object => object,
        BlobShape::JsonString => JsonValue::String(serde_json::to_string(&object).unwrap()),
    };
    let hydration = hydration(FakeStore::returning(Some(stored)), cipher);
    let mut server = with_client_secret(server(), "leftover-secret");
    assert!(
        hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(server.fields.client_id.as_deref(), Some("stored-client"));
    assert_eq!(client_secret(&server).as_deref(), Some("stored-secret"));
    assert_eq!(
        server.fields.token_endpoint_auth_method,
        Some(MCPTokenEndpointAuthMethod::ClientSecretPost)
    );
    assert_eq!(
        hydration.store.last_server.lock().unwrap().as_deref(),
        Some("srv")
    );
}

#[tokio::test]
async fn a_stored_client_without_a_secret_clears_a_leftover_secret() {
    let cipher = cipher_with(Some(SALT_KEY));
    let stored = blob(&[(
        "client_id",
        JsonValue::String(sealed(&cipher, "stored-client")),
    )]);
    let hydration = hydration(FakeStore::returning(Some(stored)), cipher);
    let mut server = with_client_secret(server(), "leftover-secret");
    assert!(
        hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(server.fields.client_id.as_deref(), Some("stored-client"));
    assert_eq!(server.fields.client_secret, None);
}

#[rstest]
#[case::null(JsonValue::Null)]
#[case::empty_string(JsonValue::String(String::new()))]
#[case::empty_object(JsonValue::Object(BTreeMap::new()))]
#[case::empty_array(JsonValue::Array(Vec::new()))]
#[tokio::test]
async fn an_empty_blob_leaves_the_server_alone(#[case] stored: JsonValue) {
    let hydration = hydration(
        FakeStore::returning(Some(stored)),
        cipher_with(Some(SALT_KEY)),
    );
    let mut server = with_client_secret(server(), "leftover-secret");
    assert!(
        !hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(server.fields.client_id, None);
    assert_eq!(client_secret(&server).as_deref(), Some("leftover-secret"));
}

#[tokio::test]
async fn a_blob_without_a_client_id_leaves_the_server_alone() {
    let stored = blob(&[(
        "client_secret",
        JsonValue::String("stored-secret".to_owned()),
    )]);
    let hydration = hydration(
        FakeStore::returning(Some(stored)),
        cipher_with(Some(SALT_KEY)),
    );
    let mut server = server();
    assert!(
        !hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(server.fields.client_id, None);
}

#[tokio::test]
async fn a_blob_of_the_wrong_shape_leaves_the_server_alone() {
    let stored: JsonValue = serde_json::from_str(r#"{"client_id": 5}"#).unwrap();
    let hydration = hydration(
        FakeStore::returning(Some(stored)),
        cipher_with(Some(SALT_KEY)),
    );
    let mut server = server();
    assert!(
        !hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(server.fields.client_id, None);
}

#[tokio::test]
async fn a_plaintext_client_id_is_kept_as_stored() {
    let stored = blob(&[("client_id", JsonValue::String("plain-client".to_owned()))]);
    let hydration = hydration(
        FakeStore::returning(Some(stored)),
        cipher_with(Some(SALT_KEY)),
    );
    let mut server = server();
    assert!(
        hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(server.fields.client_id.as_deref(), Some("plain-client"));
}

#[tokio::test]
async fn a_client_id_sealed_under_another_key_is_kept_as_stored() {
    let ciphertext = sealed(&cipher_with(Some("another-salt-key")), "stored-client");
    let stored = blob(&[("client_id", JsonValue::String(ciphertext.clone()))]);
    let hydration = hydration(
        FakeStore::returning(Some(stored)),
        cipher_with(Some(SALT_KEY)),
    );
    let mut server = server();
    assert!(
        hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(
        server.fields.client_id.as_deref(),
        Some(ciphertext.as_str())
    );
}

#[rstest]
#[case::plaintext(false)]
#[case::sealed(true)]
#[tokio::test]
async fn an_empty_client_id_leaves_the_server_alone(#[case] seal: bool) {
    let cipher = cipher_with(Some(SALT_KEY));
    let text = if seal {
        sealed(&cipher, "")
    } else {
        String::new()
    };
    let stored = blob(&[("client_id", JsonValue::String(text))]);
    let hydration = hydration(FakeStore::returning(Some(stored)), cipher);
    let mut server = with_client_secret(server(), "leftover-secret");
    assert!(
        !hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(server.fields.client_id, None);
    assert_eq!(client_secret(&server).as_deref(), Some("leftover-secret"));
}

#[tokio::test]
async fn an_unknown_token_endpoint_auth_method_is_cleared_and_logged_at_debug() {
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let cipher = cipher_with(Some(SALT_KEY));
    let stored = blob(&[
        (
            "client_id",
            JsonValue::String(sealed(&cipher, "stored-client")),
        ),
        (
            "token_endpoint_auth_method",
            JsonValue::String("private_key_jwt".to_owned()),
        ),
    ]);
    let hydration = hydration(FakeStore::returning(Some(stored)), cipher);
    let mut server = server();
    assert!(
        hydration
            .hydrate_config_server_dcr_client(&mut server)
            .await
    );
    assert_eq!(server.fields.client_id.as_deref(), Some("stored-client"));
    assert_eq!(server.fields.token_endpoint_auth_method, None);
    assert!(
        logs.messages()
            .iter()
            .any(|message| message.contains("private_key_jwt"))
    );
}

#[rstest]
#[case::missing(None, "https://proxy.test/callback", false)]
#[case::empty(Some(Vec::new()), "https://proxy.test/callback", false)]
#[case::registered(
    Some(vec!["https://proxy.test/callback"]),
    "https://proxy.test/callback",
    false
)]
#[case::stale(Some(vec!["https://old.test/callback"]), "https://proxy.test/callback", true)]
fn redirect_uri_not_registered_follows_the_recorded_list(
    #[case] redirect_uris: Option<Vec<&str>>,
    #[case] current: &str,
    #[case] expected: bool,
) {
    let recorded = redirect_uris.map(|uris| uris.iter().map(|uri| (*uri).to_owned()).collect());
    assert_eq!(
        redirect_uri_not_registered(&credentials(recorded), current),
        expected
    );
}
