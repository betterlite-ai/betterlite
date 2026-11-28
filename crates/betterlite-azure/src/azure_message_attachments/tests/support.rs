#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::ThreadMessageData;
use http::HeaderMap;
use secrecy::SecretString;

use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::operations::{AssistantsApi, ClientRequest};
use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::azure_message_attachments::operations::MessagesApi;
use crate::azure_urls::AuthParams;
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::endpoints::projection::{CallableProvider, LiteLlmParams, SessionKind};
use crate::oidc_exchange::OidcExchangeInput;

pub(super) const KEY: &str = "sk-fixture";
pub(super) const MESSAGES_URL: &str =
    "https://example.test/openai/threads/thread_1/messages?api-version=2024-02-01";
pub(super) const ERROR_BODY: &str =
    r#"{"error":{"message":"nope","type":"invalid_request_error"}}"#;
pub(super) const MESSAGE: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"user","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"completed"}"#;
pub(super) const MESSAGE_IN_PROGRESS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"user","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"in_progress"}"#;
pub(super) const MESSAGE_NO_STATUS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"user","content":[{"type":"text","text":{"value":"hi","annotations":[]}}]}"#;
pub(super) const MESSAGE_NULL_STATUS: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"user","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":null}"#;
pub(super) const MESSAGE_EXTRA: &str = r#"{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"user","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"completed","marker":"kept"}"#;
pub(super) const MESSAGE_DATA: &str = r#"{"role":"user","content":"hi"}"#;
pub(super) const MESSAGE_ATTACHMENTS: &str = r#"{"role":"user","content":"hi","attachments":[{"file_id":"file-1","tools":[{"type":"code_interpreter"}]}],"metadata":{"team":"core"}}"#;
pub(super) const PAGE: &str =
    r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
pub(super) const PAGE_WITH_MESSAGE: &str = r#"{"object":"list","data":[{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1","role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"completed","marker":"kept"}],"first_id":"msg_1","last_id":"msg_1","has_more":true}"#;

// The headers the helpers compare, kept by their wire names.
pub(super) type Headers = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) deadline: Duration,
    pub(super) headers: Headers,
    pub(super) body: Option<ThreadMessageData>,
}

#[derive(Clone, Copy)]
pub(super) enum Auth<'text> {
    Key(&'text str),
    Bearer(&'text str),
    Both { key: &'text str, bearer: &'text str },
}

#[derive(Default)]
pub(super) struct Recorder {
    replies: Mutex<VecDeque<Result<(u16, String), TransportError>>>,
    seen: Mutex<Vec<Seen>>,
}

impl Recorder {
    pub(super) fn queue(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap()
            .push_back(Ok((status, body.to_owned())));
    }

    pub(super) fn fail_next(&self) {
        self.replies
            .lock()
            .unwrap()
            .push_back(Err(TransportError::Closed));
    }

    pub(super) fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn headers_of(headers: &HeaderMap) -> Headers {
    let mut kept = Headers::new();
    for name in [
        "accept",
        "api-key",
        "authorization",
        "content-type",
        "openai-beta",
    ] {
        if let Some(value) = headers.get(name) {
            kept.insert(name.to_owned(), value.to_str().unwrap().to_owned());
        }
    }
    kept
}

fn recorded(request: &OutboundRequest) -> Seen {
    let mut url = request.url.clone();
    merge_query(&mut url, &request.query);
    let body = match &request.body {
        RequestBody::Json(bytes) => {
            Some(serde_json::from_slice::<ThreadMessageData>(bytes).unwrap())
        }
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
    };
    let mut headers = headers_of(&request.headers);
    // The shared transport adds this header to a JSON body itself.
    if body.is_some() {
        headers
            .entry(String::from("content-type"))
            .or_insert_with(|| String::from("application/json"));
    }
    Seen {
        method: request.method.as_str().to_owned(),
        url: url.as_str().to_owned(),
        deadline: request.deadline,
        headers,
        body,
    }
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let seen = recorded(&request);
        self.seen.lock().unwrap().push(seen);
        let reply = self.replies.lock().unwrap().pop_front().unwrap();
        match reply {
            Ok((status, text)) => ready(Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: text.into_bytes(),
            })),
            Err(error) => ready(Err(error)),
        }
    }
}

pub(super) fn harness(status: u16, body: &str) -> (MessagesApi<Recorder>, Arc<Recorder>) {
    let transport = Arc::new(Recorder::default());
    transport.queue(status, body);
    let api = MessagesApi {
        assistants: AssistantsApi {
            transport: Arc::clone(&transport),
        },
    };
    (api, transport)
}

// What the Azure SDK sends for a method once auth and the API version have been applied.
pub(super) fn wire(method: &str, url: &str, body: Option<&str>, auth: Auth<'_>) -> Seen {
    let mut headers = Headers::new();
    headers.insert(String::from("accept"), String::from("application/json"));
    headers.insert(String::from("openai-beta"), String::from("assistants=v2"));
    if method != "GET" {
        headers.insert(
            String::from("content-type"),
            String::from("application/json"),
        );
    }
    match auth {
        Auth::Key(key) => {
            headers.insert(String::from("api-key"), key.to_owned());
        }
        Auth::Bearer(bearer) => {
            headers.insert(String::from("authorization"), format!("Bearer {bearer}"));
        }
        Auth::Both { key, bearer } => {
            headers.insert(String::from("api-key"), key.to_owned());
            headers.insert(String::from("authorization"), format!("Bearer {bearer}"));
        }
    }
    Seen {
        method: method.to_owned(),
        url: url.to_owned(),
        deadline: Duration::from_secs(600),
        headers,
        body: body.map(|text| serde_json::from_str::<ThreadMessageData>(text).unwrap()),
    }
}

pub(super) fn data(json: &str) -> ThreadMessageData {
    serde_json::from_str(json).unwrap()
}

// A caller that holds no Azure AD token source: every token request refuses.
pub(super) struct NoSources;

fn refuse<T>(kind: &str) -> Result<T, SourceFailure> {
    Err(SourceFailure {
        class: String::from("CredentialUnavailableError"),
        status: None,
        message: format!("no {kind} source is configured"),
    })
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("client secret")
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("username and password")
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("SDK refresh")
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        refuse("OIDC")
    }
}

pub(super) struct FixedToken(pub(super) &'static str);

impl CallableTokenProvider for FixedToken {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::Text(SecretString::from(self.0)))
    }
}

pub(super) struct EmptyToken;

impl CallableTokenProvider for EmptyToken {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::Text(SecretString::from("")))
    }
}

pub(super) struct FailingToken;

impl CallableTokenProvider for FailingToken {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Err(ProviderFailure {
            class: String::from("ClientAuthenticationError"),
            message: String::from("the provider rejected the request"),
        })
    }
}

pub(super) struct Fixture {
    pub(super) settings: Settings,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) sources: NoSources,
    pub(super) params: LiteLlmParams,
    pub(super) provider: Option<CallableProvider>,
    pub(super) api_key: Option<SecretString>,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) azure_ad_token: Option<SecretString>,
    pub(super) timeout: f64,
    pub(super) max_retries: Option<i64>,
    pub(super) refresh_enabled: bool,
    pub(super) session: SessionKind,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            settings: Settings {
                default_max_retries: 2,
                azure_default_api_version: String::from("2025-02-01-preview"),
            },
            environment: BTreeMap::new(),
            sources: NoSources,
            params: LiteLlmParams {
                azure_ad_token: None,
                auth: AuthParams::default(),
                max_retries: None,
                timeout: None,
            },
            provider: None,
            api_key: Some(SecretString::from(KEY)),
            api_base: Some(String::from("https://example.test")),
            api_version: Some(String::from("2024-02-01")),
            azure_ad_token: None,
            timeout: 600.0,
            max_retries: None,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
        }
    }
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn request(&self) -> ClientRequest<'_> {
        ClientRequest {
            params: self.params.clone(),
            provider: self.provider.clone(),
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            api_version: self.api_version.clone(),
            azure_ad_token: self.azure_ad_token.clone(),
            timeout: self.timeout,
            max_retries: self.max_retries,
            refresh_enabled: self.refresh_enabled,
            session: self.session,
            settings: &self.settings,
            environment: &self.environment,
            sources: &self.sources,
        }
    }
}

pub(super) fn client_handle(
    kind: ClientKind,
    transport: Arc<Recorder>,
) -> AzureAssistantsClient<'static, Recorder> {
    AzureAssistantsClient {
        kind,
        inputs: None,
        transport,
        base_url: String::from("https://supplied.test/openai"),
        api_version: String::from("2023-05-01"),
        key: Some(SecretString::from("supplied-key")),
        token: None,
        deadline: Duration::from_secs(42),
    }
}
