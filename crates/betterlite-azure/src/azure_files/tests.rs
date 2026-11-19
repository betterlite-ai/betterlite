#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt;
use std::future::{Future, ready};
use std::str;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{
    BinaryContent, FileContentRequest, FileCreate, FileDeleted, FileObject, FilePage,
};
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use betterlite_openai::files::{DEFAULT_TIMEOUT, FilesClient};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use url::Url;

use super::client::auth::{AuthKind, FileAuth};
use super::client::{ClientMaterializer, ClientSettings, Connection, Opened, SelectedClient};
use super::error::AzureFilesError;
use super::{Operation, Reply, call};
use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::endpoints::get_client::ClientCache;
use crate::endpoints::projection::{
    CallableProvider, ClientIdentity, ClientKind, ClientOutcome, LiteLlmParams, SessionKind,
    SuppliedClient,
};
use crate::oidc_exchange::OidcExchangeInput;

const ENDPOINT: &str = "https://fixture.test";
const KEY: &str = "key-1";
const VERSION: &str = "2024-10-21";
const SUPPLIED_ENDPOINT: &str = "https://supplied.test";
const SUPPLIED_KEY: &str = "supplied-key";
const SUPPLIED_VERSION: &str = "2025-01-01";

const DEBUG_CREATE: &str = "message=create_file_data";
const DEBUG_RESPONSE: &str = "message=create_file_response";

const FILE_JSON: &str = r#"{"id":"file-1","object":"file","bytes":5,"created_at":1700000000,"filename":"notes.txt","purpose":"batch","status":"processed"}"#;
const PAGE_JSON: &str = r#"{"object":"list","data":[{"id":"file-1","object":"file","bytes":5,"created_at":1700000000,"filename":"notes.txt","purpose":"batch","status":"processed"}],"has_more":false}"#;
const STATUS_ERROR: &str = r#"{"error":{"code":"not_found","message":"no such file","param":"file_id","type":"invalid_request_error"}}"#;

type Calls = Arc<Mutex<Vec<OutboundRequest>>>;
type Seen = Arc<Mutex<Vec<ClientKind>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Client,
    Create,
    Content,
    Retrieve,
    List,
    Delete,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Supplied {
    OpenAi,
    OpenAiAsync,
    Azure,
    AzureAsync,
}

impl Supplied {
    const ALL: [Self; 4] = [
        Self::OpenAi,
        Self::OpenAiAsync,
        Self::Azure,
        Self::AzureAsync,
    ];

    const fn kind(self) -> ClientKind {
        match self {
            Self::OpenAi => ClientKind::OpenAi,
            Self::OpenAiAsync => ClientKind::AsyncOpenAi,
            Self::Azure => ClientKind::AzureOpenAi,
            Self::AzureAsync => ClientKind::AsyncAzureOpenAi,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum BuilderMode {
    #[default]
    Default,
    None,
}

#[derive(Clone)]
enum ProviderReply {
    Text(String),
    NonText(String),
    Failure,
}

struct FakeProvider(ProviderReply);

impl CallableTokenProvider for FakeProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        match &self.0 {
            ProviderReply::Text(text) => Ok(TokenReply::Text(SecretString::from(text.clone()))),
            ProviderReply::NonText(type_name) => Ok(TokenReply::NonText {
                type_name: type_name.clone(),
            }),
            ProviderReply::Failure => Err(ProviderFailure {
                class: "ValueError".to_owned(),
                message: "the token call failed".to_owned(),
            }),
        }
    }
}

fn callable(reply: ProviderReply) -> CallableProvider {
    CallableProvider {
        provider: Arc::new(FakeProvider(reply)),
        identity: 7,
    }
}

struct TestSources;

fn source_failure() -> SourceFailure {
    SourceFailure {
        class: "Unavailable".to_owned(),
        status: None,
        message: "the fixture has no credential sources".to_owned(),
    }
}

impl CredentialSources for TestSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(source_failure())
    }
}

struct TestCache {
    hit: Option<ClientKind>,
}

impl ClientCache for TestCache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        self.hit
    }

    fn set(&mut self, _identity: ClientIdentity, _kind: ClientKind, _owned: bool) {}
}

#[derive(Clone)]
struct Planned {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: Vec<u8>,
}

fn reply(status: u16, body: &str) -> Planned {
    Planned {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn json_reply(body: &str) -> Planned {
    let mut reply = reply(200, body);
    reply.headers.push(("content-type", "application/json"));
    reply
}

struct Fake {
    planned: Option<Planned>,
    calls: Calls,
    fail: bool,
}

impl Fake {
    fn record(&self, request: OutboundRequest) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    fn answer(&self) -> Result<UpstreamResponse, TransportError> {
        let planned = self
            .planned
            .as_ref()
            .filter(|_planned| !self.fail)
            .ok_or(TransportError::HeadersTooLarge { limit: 0 })?;
        let mut headers = HeaderMap::new();
        for (name, value) in &planned.headers {
            headers.append(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        Ok(UpstreamResponse {
            status: planned.status,
            headers,
            body: planned.body.clone(),
        })
    }
}

impl FileTransport for Fake {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        ready(self.answer())
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        self.record(request);
        ready(Err(TransportError::HeadersTooLarge { limit: 0 }))
    }
}

struct Materializer {
    mode: BuilderMode,
    seen: Seen,
}

impl ClientMaterializer for Materializer {
    fn materialize(
        &self,
        outcome: &ClientOutcome<'_>,
    ) -> Result<Option<SelectedClient>, AzureFilesError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(outcome.kind);
        if self.mode == BuilderMode::None {
            return Ok(None);
        }
        let (base_url, auth, api_version) = match outcome.kind {
            ClientKind::AzureOpenAi | ClientKind::AsyncAzureOpenAi => (
                format!("{SUPPLIED_ENDPOINT}/openai/"),
                FileAuth::ApiKey(SecretString::from(SUPPLIED_KEY)),
                Some(SUPPLIED_VERSION.to_owned()),
            ),
            ClientKind::OpenAi | ClientKind::AsyncOpenAi | ClientKind::Other => (
                format!("{SUPPLIED_ENDPOINT}/v1/"),
                FileAuth::Bearer(SecretString::from(SUPPLIED_KEY)),
                None,
            ),
        };
        let files = FilesClient {
            api_key: SecretString::from(SUPPLIED_KEY),
            base_url: Url::parse(&base_url)
                .map_err(|source| AzureFilesError::BaseUrl { source })?,
            organization: None,
            timeout: DEFAULT_TIMEOUT,
            max_retries: 0,
        };
        Ok(Some(SelectedClient::new(
            files,
            &auth,
            api_version,
            outcome.kind,
        )?))
    }
}

fn supplied_client(kind: Supplied) -> SuppliedClient {
    let custom_query = match kind.kind() {
        ClientKind::AzureOpenAi | ClientKind::AsyncAzureOpenAi => Some(BTreeMap::from([(
            "api-version".to_owned(),
            SUPPLIED_VERSION.to_owned(),
        )])),
        ClientKind::OpenAi | ClientKind::AsyncOpenAi | ClientKind::Other => None,
    };
    SuppliedClient {
        kind: kind.kind(),
        custom_query,
    }
}

#[derive(Clone)]
struct Step {
    action: Action,
    is_async: bool,
    client: Option<Supplied>,
    builder: BuilderMode,
    reply: Option<Planned>,
    settings: ClientSettings,
    params: LiteLlmParams,
    data: Option<FileCreate>,
    content: Option<FileContentRequest>,
    file_id: Option<String>,
    purpose: Option<String>,
    provider: Option<CallableProvider>,
    cache_hit: Option<ClientKind>,
    fail: bool,
}

fn settings() -> ClientSettings {
    ClientSettings {
        api_key: Some(SecretString::from(KEY)),
        api_base: Some(ENDPOINT.to_owned()),
        api_version: Some(VERSION.to_owned()),
    }
}

fn step(action: Action) -> Step {
    Step {
        action,
        is_async: false,
        client: None,
        builder: BuilderMode::Default,
        reply: None,
        settings: settings(),
        params: LiteLlmParams::default(),
        data: None,
        content: None,
        file_id: None,
        purpose: None,
        provider: None,
        cache_hit: None,
        fail: false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClientView {
    api_key: String,
    base_url: String,
    api_version: Option<String>,
    timeout: Duration,
    max_retries: u32,
    kind: ClientKind,
    query: Vec<(String, String)>,
    auth: AuthKind,
}

impl ClientView {
    fn of(client: &SelectedClient) -> Self {
        Self {
            api_key: client.files.api_key.expose_secret().to_owned(),
            base_url: client.files.base_url.to_string(),
            api_version: client.api_version.clone(),
            timeout: client.files.timeout,
            max_retries: client.files.max_retries,
            kind: client.kind,
            query: client.query(),
            auth: client.auth_kind(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Raised {
    class: &'static str,
    message: Option<String>,
    status: Option<u16>,
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Object(FileObject),
    Content(BinaryContent),
    Page(FilePage),
    Deleted(FileDeleted),
    Client(ClientView),
    Raised(Raised),
}

struct Observation {
    outcome: Outcome,
    calls: Vec<OutboundRequest>,
    messages: Vec<String>,
    seen: Vec<ClientKind>,
}

async fn perform(
    prepared: Result<Opened, AzureFilesError>,
    step: &Step,
    transport: &Fake,
) -> Outcome {
    let opened = match prepared {
        Ok(opened) => opened,
        Err(error) => return Outcome::Raised(raised(&error)),
    };
    let Some(operation) = file_operation(step) else {
        return Outcome::Client(ClientView::of(&opened.client));
    };
    call(&opened, transport, operation)
        .await
        .map_or_else(|error| Outcome::Raised(raised(&error)), reply_outcome)
}

fn file_operation(step: &Step) -> Option<Operation<'_>> {
    match step.action {
        Action::Client => None,
        Action::Create => {
            let Some(data) = step.data.as_ref() else {
                panic!("a create step carries data");
            };
            Some(Operation::Create(data))
        }
        Action::Content => {
            let Some(content) = step.content.as_ref() else {
                panic!("a content step carries a request");
            };
            Some(Operation::Content(content))
        }
        Action::Retrieve => {
            let Some(file_id) = step.file_id.as_deref() else {
                panic!("a retrieve step carries a file id");
            };
            Some(Operation::Retrieve(file_id))
        }
        Action::List => Some(Operation::List(step.purpose.as_deref())),
        Action::Delete => {
            let Some(file_id) = step.file_id.as_deref() else {
                panic!("a delete step carries a file id");
            };
            Some(Operation::Delete(file_id))
        }
    }
}

fn reply_outcome(reply: Reply) -> Outcome {
    match reply {
        Reply::Object(object) => Outcome::Object(object),
        Reply::Content(content) => Outcome::Content(content),
        Reply::Page(page) => Outcome::Page(page),
        Reply::Deleted(deleted) => Outcome::Deleted(deleted),
    }
}

fn raised(error: &AzureFilesError) -> Raised {
    match error {
        AzureFilesError::MissingCredentials => report("OpenAIError", error),
        AzureFilesError::Files { source } => raised_files(source, error),
        AzureFilesError::ClientMissing
        | AzureFilesError::NotAsyncClient
        | AzureFilesError::UnableToHandleAuth
        | AzureFilesError::TokenNotString { .. }
        | AzureFilesError::TokenProvider { .. }
        | AzureFilesError::MissingBaseUrl
        | AzureFilesError::BaseUrl { .. }
        | AzureFilesError::Header { .. }
        | AzureFilesError::Timeout { .. }
        | AzureFilesError::Endpoint { .. } => report("ValueError", error),
    }
}

fn report(class: &'static str, error: &AzureFilesError) -> Raised {
    Raised {
        class,
        message: Some(error.to_string()),
        status: None,
        error_type: None,
        param: None,
        code: None,
    }
}

fn raised_files(source: &FilesError, error: &AzureFilesError) -> Raised {
    if let FilesError::Status(status) = source {
        return Raised {
            class: "APIStatusError",
            message: Some(status.message.clone()),
            status: Some(status.status),
            error_type: status.error_type.clone(),
            param: status.param.clone(),
            code: status.code.clone(),
        };
    }
    if matches!(source, FilesError::Decode { .. }) {
        return Raised {
            class: "ValidationError",
            message: None,
            status: None,
            error_type: None,
            param: None,
            code: None,
        };
    }
    report("ValueError", error)
}

struct Fixtures {
    endpoints: EndpointSettings,
    environment: BTreeMap<String, String>,
    sources: TestSources,
}

fn connect<'borrow, 'sources>(
    step: &Step,
    fixtures: &'sources Fixtures,
    cache: &'borrow mut TestCache,
    refresh: &'borrow mut RefreshProviderCache<'sources>,
    materializer: &'borrow Materializer,
) -> Connection<'borrow, 'sources, Materializer> {
    Connection {
        settings: step.settings.clone(),
        params: step.params.clone(),
        provider: step.provider.clone(),
        supplied: step.client.map(supplied_client),
        is_async: step.is_async,
        endpoints: &fixtures.endpoints,
        environment: &fixtures.environment,
        sources: &fixtures.sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache,
        refresh,
        materializer,
    }
}

fn run(step: &Step) -> Observation {
    let calls: Calls = Arc::default();
    let seen: Seen = Arc::default();
    let fixtures = Fixtures {
        endpoints: EndpointSettings::default(),
        environment: BTreeMap::new(),
        sources: TestSources,
    };
    let mut cache = TestCache {
        hit: step.cache_hit,
    };
    let mut refresh = RefreshProviderCache::new(&fixtures.sources);
    let runtime = Builder::new_current_thread()
        .build()
        .expect("a test runtime builds");
    let (outcome, messages) = {
        let transport = Fake {
            planned: step.reply.clone(),
            calls: Arc::clone(&calls),
            fail: step.fail,
        };
        let materializer = Materializer {
            mode: step.builder,
            seen: Arc::clone(&seen),
        };
        let connection = connect(step, &fixtures, &mut cache, &mut refresh, &materializer);
        capture(|| runtime.block_on(perform(connection.open(), step, &transport)))
    };
    observation(outcome, calls, messages, seen)
}

fn observation(outcome: Outcome, calls: Calls, messages: Vec<String>, seen: Seen) -> Observation {
    Observation {
        outcome,
        calls: Arc::into_inner(calls)
            .expect("the fixture holds the calls")
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner),
        messages,
        seen: Arc::into_inner(seen)
            .expect("the fixture holds the seen kinds")
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner),
    }
}

fn only_call(observation: &Observation) -> &OutboundRequest {
    assert_eq!(observation.calls.len(), 1, "one request is sent");
    &observation.calls[0]
}

fn header<'request>(request: &'request OutboundRequest, name: &str) -> Option<&'request str> {
    request.headers.get(name)?.to_str().ok()
}

fn part_view(request: &OutboundRequest) -> Vec<(String, Option<String>, Option<String>, String)> {
    let RequestBody::Multipart { parts, .. } = &request.body else {
        return Vec::new();
    };
    parts
        .iter()
        .map(|part| {
            (
                part.name.clone(),
                part.file_name.clone(),
                part.content_type.clone(),
                str::from_utf8(&part.data).unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[derive(Clone, Default)]
struct Messages(Arc<Mutex<Vec<String>>>);

struct MessageVisitor(Vec<String>);

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push(format!("{}={value}", field.name()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.push(format!("{}={value}", field.name()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.push(format!("{}={value}", field.name()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }
}

impl<S: tracing::Subscriber> Layer<S> for Messages {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let mut visitor = MessageVisitor(Vec::new());
        event.record(&mut visitor);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(visitor.0.join(";"));
    }
}

fn capture<T, F>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let messages = Messages::default();
    let subscriber = tracing_subscriber::registry().with(messages.clone());
    let result = with_default(subscriber, run);
    let captured = Arc::into_inner(messages.0)
        .expect("the capture holds the messages")
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    (result, captured)
}

mod cases;
mod inputs;
