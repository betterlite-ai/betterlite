#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::ready;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::Value;

use crate::azure_assistant_definitions::operations::{AssistantsApi, ClientRequest};
use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::azure_urls::AuthParams;
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::endpoints::projection::{CallableProvider, LiteLlmParams, SessionKind};
use crate::oidc_exchange::OidcExchangeInput;

pub(super) const KEY: &str = "sk-fixture";
pub(super) const ASSISTANTS_URL: &str =
    "https://example.test/openai/assistants?api-version=2024-02-01";
pub(super) const DELETE_URL: &str =
    "https://example.test/openai/assistants/asst_1?api-version=2024-02-01";
pub(super) const ERROR_BODY: &str =
    r#"{"error":{"message":"nope","type":"invalid_request_error"}}"#;
pub(super) const PAGE: &str =
    r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
pub(super) const PAGE_EXTRA: &str =
    r#"{"object":"list","data":[],"has_more":false,"marker":"kept"}"#;
pub(super) const ASSISTANT: &str = r#"{"id":"asst_1","object":"assistant","created_at":7,"model":"gpt-4o","tools":[],"marker":"kept"}"#;
pub(super) const DELETED: &str = r#"{"id":"asst_1","deleted":true,"object":"assistant.deleted"}"#;
pub(super) const CREATE_BODY: &str = r#"{"model":"gpt-4o","name":"Helper","instructions":"answer","description":null,"metadata":{"team":"a"},"tools":[{"type":"code_interpreter"},{"type":"file_search","file_search":{"max_num_results":4}},{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}],"tool_resources":{"code_interpreter":{"file_ids":["file-1"]}},"temperature":0.2,"top_p":0.9,"response_format":{"type":"json_object"},"reasoning_effort":"high"}"#;

// The headers the helpers compare, kept by their wire names.
pub(super) type Headers = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) deadline: Duration,
    pub(super) headers: Headers,
    pub(super) body: Option<Value>,
}

pub(super) enum Auth<'text> {
    None,
    Key(&'text str),
    Bearer(&'text str),
    Both { key: &'text str, bearer: &'text str },
}

#[derive(Default)]
pub(super) struct Recorder {
    replies: Mutex<VecDeque<Result<(u16, String), TransportError>>>,
    seen: Mutex<Vec<Seen>>,
    shown: Mutex<Vec<String>>,
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

    pub(super) fn debug_texts(&self) -> Vec<String> {
        self.shown.lock().unwrap().clone()
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
        RequestBody::Json(bytes) => Some(serde_json::from_slice(bytes).unwrap()),
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
        self.shown.lock().unwrap().push(format!("{request:?}"));
        let seen = recorded(&request);
        self.seen.lock().unwrap().push(seen);
        match self.replies.lock().unwrap().pop_front().unwrap() {
            Ok((status, text)) => ready(Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: text.into_bytes(),
            })),
            Err(error) => ready(Err(error)),
        }
    }
}

pub(super) fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

pub(super) fn harness(status: u16, body: &str) -> (AssistantsApi<Recorder>, Arc<Recorder>) {
    let transport = Arc::new(Recorder::default());
    transport.queue(status, body);
    let api = AssistantsApi {
        transport: Arc::clone(&transport),
    };
    (api, transport)
}

pub(super) fn failing_harness() -> (AssistantsApi<Recorder>, Arc<Recorder>) {
    let transport = Arc::new(Recorder::default());
    transport.fail_next();
    let api = AssistantsApi {
        transport: Arc::clone(&transport),
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
        Auth::None => {}
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
        body: body.map(parse),
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

#[derive(Default)]
pub(super) struct ScriptedSources {
    replies: Mutex<VecDeque<Result<Arc<dyn CallableTokenProvider>, SourceFailure>>>,
    scopes: Mutex<Vec<String>>,
}

impl ScriptedSources {
    pub(super) fn script(&self, provider: Arc<dyn CallableTokenProvider>) {
        self.replies.lock().unwrap().push_back(Ok(provider));
    }

    pub(super) fn fail_next(&self, failure: SourceFailure) {
        self.replies.lock().unwrap().push_back(Err(failure));
    }

    pub(super) fn refresh_scopes(&self) -> Vec<String> {
        self.scopes.lock().unwrap().clone()
    }
}

fn unscripted(source: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("ClientAuthenticationError"),
        status: None,
        message: format!("{source} is not scripted"),
    }
}

impl CredentialSources for ScriptedSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted("client_secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted("username_password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        self.scopes.lock().unwrap().push(scope.to_owned());
        match self.replies.lock().unwrap().pop_front() {
            Some(Ok(provider)) => Ok(provider),
            Some(Err(failure)) => Err(failure),
            None => Err(unscripted("sdk_refresh")),
        }
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unscripted("oidc"))
    }
}

pub(super) struct Fixture {
    pub(super) settings: Settings,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) sources: ScriptedSources,
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

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            settings: Settings {
                default_max_retries: 2,
                azure_default_api_version: String::from("2025-02-01-preview"),
            },
            environment: BTreeMap::new(),
            sources: ScriptedSources::default(),
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
