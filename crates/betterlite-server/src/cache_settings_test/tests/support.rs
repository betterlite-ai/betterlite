#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use betterlite_auth::auth_identity::records::CredentialRef;
use betterlite_auth::auth_identity::records::NetworkContext;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::records::SourceKey;
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::auth_identity::roles::PrincipalType;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_redis::redis_live_client::LiveClientFactory;
use betterlite_secrets::secret_codec::Algorithm;
use betterlite_secrets::secret_codec::CipherSettings;
use betterlite_secrets::secret_codec::OsNonces;
use betterlite_secrets::secret_codec::StoredValue;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde_json::Map;
use serde_json::Value;
use tokio::io::AsyncBufRead;
use tokio::io::AsyncBufReadExt as _;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::io::BufReader;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::cache_settings_test::route::CacheTestState;
use crate::cache_settings_test::route::router;
use crate::cache_settings_test::saved::SavedCacheConfig;

pub(crate) const SAVED_SECRET: &str = "saved-redis-secret";
pub(crate) const TYPED_SECRET: &str = "typed-redis-secret";
pub(crate) const REDACTED: &str = "***REDACTED***";

const TEST_SIGNING_KEY: &str = "cache-settings-test-signing-key";

pub(crate) fn cipher() -> DbCipher {
    cipher_with(Some(SecretString::from(TEST_SIGNING_KEY)))
}

pub(crate) fn cipher_with(signing_key: Option<SecretString>) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key,
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn sealed(text: &str) -> String {
    let sealed = cipher()
        .encrypt_value(StoredValue::<()>::Text(SecretString::from(text)), None)
        .expect("the test value seals");
    match sealed {
        StoredValue::Text(value) => value.expose_secret().to_owned(),
        StoredValue::Other(()) => panic!("a string seals as text"),
    }
}

pub(crate) enum SavedReply {
    Stored(Value),
    Missing,
    Failed,
}

pub(crate) struct FakeSaved {
    reply: SavedReply,
    calls: AtomicUsize,
}

impl FakeSaved {
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl SavedCacheConfig for FakeSaved {
    async fn cache_settings(&self) -> Result<Option<Value>, TableError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.reply {
            SavedReply::Stored(value) => Ok(Some(value.clone())),
            SavedReply::Missing => Ok(None),
            SavedReply::Failed => Err(TableError::MissingRow),
        }
    }
}

pub(crate) fn fake_saved(reply: SavedReply) -> (Arc<FakeSaved>, Option<Arc<dyn SavedCacheConfig>>) {
    let fake = Arc::new(FakeSaved {
        reply,
        calls: AtomicUsize::new(0),
    });
    let erased: Arc<dyn SavedCacheConfig> = Arc::<FakeSaved>::clone(&fake);
    (fake, Some(erased))
}

pub(crate) fn state(saved: Option<Arc<dyn SavedCacheConfig>>) -> CacheTestState {
    CacheTestState {
        saved,
        cipher: Arc::new(cipher()),
        redis_factory: Arc::new(LiveClientFactory::new(None)),
        environment: BTreeMap::new(),
    }
}

pub(crate) fn state_with_environment(environment: BTreeMap<String, String>) -> CacheTestState {
    CacheTestState {
        saved: None,
        cipher: Arc::new(cipher()),
        redis_factory: Arc::new(LiveClientFactory::new(None)),
        environment,
    }
}

pub(crate) fn admin() -> AuthContext {
    master_caller()
}

pub(crate) fn platform_admin() -> AuthContext {
    caller("platform-admin", vec![Role::PlatformAdmin])
}

pub(crate) fn non_admin() -> AuthContext {
    caller("internal-user", Vec::new())
}

fn caller(subject: &str, roles: Vec<Role>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(crate) fn request(settings: Value) -> String {
    serde_json::to_string(&Map::from_iter([("cache_settings".to_owned(), settings)]))
        .expect("the request encodes")
}

pub(crate) async fn post(
    state: CacheTestState,
    auth: Option<AuthContext>,
    body: &str,
) -> (StatusCode, String) {
    let application = router(state);
    let mut request = Request::builder()
        .method("POST")
        .uri("/cache/settings/test")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .expect("the test request builds");
    if let Some(context) = auth {
        request.extensions_mut().insert(context);
    }
    let response = application
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("the body is utf-8"),
    )
}

pub(crate) fn parse(body: &str) -> Value {
    serde_json::from_str(body).expect("the text is JSON")
}

pub(crate) struct FakeRedis {
    address: SocketAddr,
    commands: Arc<Mutex<Vec<Recorded>>>,
    connections: Arc<AtomicUsize>,
    tasks: JoinSet<()>,
}

#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    pub(crate) name: String,
    pub(crate) arguments: Vec<String>,
}

impl Drop for FakeRedis {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

impl FakeRedis {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral listener");
        let address = listener.local_addr().expect("the listener address");
        let commands = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(
            listener,
            Arc::clone(&commands),
            Arc::clone(&connections),
        ));
        Self {
            address,
            commands,
            connections,
            tasks,
        }
    }

    pub(crate) fn host(&self) -> String {
        self.address.ip().to_string()
    }

    pub(crate) const fn port(&self) -> u16 {
        self.address.port()
    }

    pub(crate) fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    // The password is the last AUTH argument whether or not a username was sent.
    pub(crate) fn auth_passwords(&self) -> Vec<String> {
        self.recorded()
            .into_iter()
            .filter(|recorded| recorded.name == "AUTH")
            .filter_map(|recorded| recorded.arguments.last().cloned())
            .collect()
    }

    pub(crate) fn sent_secret(&self, secret: &str) -> bool {
        self.recorded()
            .into_iter()
            .flat_map(|recorded| recorded.arguments)
            .any(|argument| argument == secret)
    }

    fn recorded(&self) -> Vec<Recorded> {
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn serve(
    listener: TcpListener,
    commands: Arc<Mutex<Vec<Recorded>>>,
    connections: Arc<AtomicUsize>,
) {
    let mut open = JoinSet::new();
    while let Ok((stream, _peer)) = listener.accept().await {
        connections.fetch_add(1, Ordering::SeqCst);
        open.spawn(serve_connection(stream, Arc::clone(&commands)));
    }
    open.abort_all();
}

async fn serve_connection(stream: TcpStream, commands: Arc<Mutex<Vec<Recorded>>>) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    while let Ok(Some(command)) = read_command(&mut reader).await {
        let reply = reply_for(&command);
        commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                name: command.name,
                arguments: command.arguments,
            });
        if write_half.write_all(&reply).await.is_err() {
            return;
        }
    }
}

struct Command {
    name: String,
    arguments: Vec<String>,
}

async fn read_command<R>(reader: &mut R) -> io::Result<Option<Command>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let Some(count_text) = line.trim().strip_prefix('*') else {
        return Ok(None);
    };
    let count: usize = count_text
        .parse()
        .map_err(|_error| io::Error::other("the fake needs an array command"))?;
    let mut arguments = Vec::with_capacity(count);
    for _ in 0..count {
        let mut header = String::new();
        if reader.read_line(&mut header).await? == 0 {
            return Ok(None);
        }
        let Some(length_text) = header.trim().strip_prefix('$') else {
            return Ok(None);
        };
        let length: usize = length_text
            .parse()
            .map_err(|_error| io::Error::other("the fake needs a bulk string"))?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer).await?;
        buffer.truncate(length);
        let argument = String::from_utf8(buffer)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        arguments.push(argument);
    }
    let Some(first) = arguments.first() else {
        return Ok(None);
    };
    let name = first.to_ascii_uppercase();
    Ok(Some(Command { name, arguments }))
}

fn reply_for(command: &Command) -> Vec<u8> {
    match command.name.as_str() {
        "PING" => b"+PONG\r\n".to_vec(),
        _ => b"+OK\r\n".to_vec(),
    }
}
