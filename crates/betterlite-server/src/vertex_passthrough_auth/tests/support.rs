use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_auth::dev_auth_mode::AuthModeFlags;
use betterlite_auth::passthrough::caller_headers::caller_filter::FilterSettings;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_vertex::credential_sources::sdk::{CredentialClock, SystemClock};
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use betterlite_vertex::token_cache::state::TokenState;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::super::auth::VertexAuthDeps;

pub(super) type LoadResult = Result<LoadedCredential, CredentialCacheError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SeenCredentials {
    None,
    Text(String),
    Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoadRequest {
    pub(super) credentials: SeenCredentials,
    pub(super) project: Option<String>,
}

pub(super) struct FakeExecutor {
    loads: Mutex<VecDeque<LoadResult>>,
    requests: Mutex<Vec<LoadRequest>>,
    load_calls: AtomicUsize,
}

impl FakeExecutor {
    pub(super) fn scripted(loads: Vec<LoadResult>) -> Self {
        Self {
            loads: Mutex::new(VecDeque::from(loads)),
            requests: Mutex::new(Vec::new()),
            load_calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn requests(&self) -> Vec<LoadRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub(super) fn load_calls(&self) -> usize {
        self.load_calls.load(Ordering::SeqCst)
    }
}

fn seen(credentials: Option<&ConfiguredCredentials>) -> SeenCredentials {
    match credentials {
        None => SeenCredentials::None,
        Some(ConfiguredCredentials::Source(_source)) => SeenCredentials::Source,
        Some(ConfiguredCredentials::Text(text)) => {
            SeenCredentials::Text(text.expose_secret().to_owned())
        }
    }
}

impl CredentialExecutor for FakeExecutor {
    fn load<'borrow>(
        &'borrow self,
        credentials: Option<&'borrow ConfiguredCredentials>,
        project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(LoadRequest {
            credentials: seen(credentials),
            project: project.map(str::to_owned),
        });
        let next = self.loads.lock().unwrap().pop_front();
        Box::pin(
            async move { next.unwrap_or_else(|| Err(CredentialCacheError::MissingCredentials)) },
        )
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(async { Err(CredentialCacheError::MissingToken) })
    }
}

pub(super) fn deps(
    executor: Arc<FakeExecutor>,
    master_key: Option<&SecretString>,
) -> VertexAuthDeps<'_> {
    let clock: Arc<dyn CredentialClock> = Arc::new(SystemClock);
    VertexAuthDeps {
        clock,
        executor,
        tasks: TaskTracker::new(),
        filters: FilterSettings {
            master_key,
            auth_mode: AuthModeFlags::default(),
            custom_auth_configured: false,
            litellm_key_header_name: None,
            endpoint_headers: &[],
        },
    }
}

pub(super) fn snapshot(token: &str, project: Option<&str>) -> CredentialSnapshot {
    CredentialSnapshot {
        token: Some(SecretString::from(token)),
        state: Some(TokenState::Fresh),
        expires_at: None,
        expired: None,
        valid: None,
        quota_project_id: None,
        project_id: project.map(str::to_owned),
    }
}

pub(super) fn loaded(token: &str, project: Option<&str>) -> LoadedCredential {
    LoadedCredential {
        credential: Some(Arc::new(snapshot(token, project))),
        project: project.map(str::to_owned),
    }
}

pub(super) fn caller(stored_api_key: Option<&str>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(stored) = stored_api_key {
        fields.insert("api_key".to_owned(), JsonData::String(stored.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the caller auth constructs")
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).expect("the header name is valid");
        let header_value =
            HeaderValue::from_bytes(value.as_bytes()).expect("the header value is valid");
        map.insert(header_name, header_value);
    }
    map
}

pub(super) fn parameters(pairs: &[(&str, Json)]) -> JsonObject {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn json_text(text: &str) -> Json {
    Json::Text(text.to_owned())
}

pub(super) fn prepared_text<'prepared>(
    headers: &'prepared IndexMap<String, SecretString>,
    name: &str,
) -> Option<&'prepared str> {
    let secret = headers.get(name)?;
    Some(secret.expose_secret())
}

pub(super) fn texts(headers: &IndexMap<String, SecretString>) -> Vec<&str> {
    headers.keys().map(String::as_str).collect()
}

pub(super) fn sorted_texts(headers: &IndexMap<String, SecretString>) -> Vec<&str> {
    let mut names = texts(headers);
    names.sort_unstable();
    names
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).expect("utf-8 log output")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
