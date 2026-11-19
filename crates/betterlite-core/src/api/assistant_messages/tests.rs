#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::records::{MessageStatus, ThreadMessage, ThreadMessagePage};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::{Builder, Runtime};

use crate::api::resource_sdk_settings::ResourceSdkSettings;

use super::connection::{MessageParams, PrepareInput, Route, prepare};
use super::error::{MessageError, MessageFailure, SetupError};
use super::options::{MessageData, MessageOptions, OptionError, get_optional_params_add_message};
use super::provider::MessageContext;
use super::service::{self, MessageRequest, Reply};

const MESSAGE_REPLY: &str = r#"{"id":"msg_a","object":"thread.message","thread_id":"thread_a","role":"user","content":[],"created_at":1}"#;
const PAGE_REPLY: &str = r#"{"object":"list","data":[],"has_more":false}"#;
const NOT_FOUND_BODY: &str = r#"{"error":{"message":"no thread","type":"invalid_request_error","param":"thread_id","code":"model_not_found"}}"#;
const AZURE_BASE: &str = "https://azure.test";
const AZURE_URL: &str =
    "https://azure.test/openai/threads/thread_a/messages?api-version=2024-05-01-preview";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body: Option<Json>,
    deadline: Duration,
}

#[derive(Default)]
struct Recorder {
    replies: Mutex<VecDeque<(u16, Vec<u8>)>>,
    seen: Mutex<Vec<Seen>>,
}

impl Recorder {
    fn queue(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap()
            .push_back((status, body.as_bytes().to_vec()));
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn headers_of(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut kept = BTreeMap::new();
    for name in [
        "accept",
        "api-key",
        "authorization",
        "openai-beta",
        "openai-organization",
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
        RequestBody::Json(bytes) => Some(serde_json::from_slice::<Json>(bytes).unwrap()),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
    };
    Seen {
        method: request.method.as_str().to_owned(),
        url: url.as_str().to_owned(),
        headers: headers_of(&request.headers),
        body,
        deadline: request.deadline,
    }
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let seen = recorded(&request);
        self.seen.lock().unwrap().push(seen);
        let (status, body) = self.replies.lock().unwrap().pop_front().unwrap();
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body,
        }))
    }
}

struct NoSources;

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
        &self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        refuse("OIDC")
    }
}

struct Fixture {
    settings: ResourceSdkSettings,
    azure: AzureSettings,
    environment: BTreeMap<String, String>,
    sources: NoSources,
    redactor: Redactor,
    transport: Arc<Recorder>,
}

fn empty_settings() -> ResourceSdkSettings {
    ResourceSdkSettings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: None,
        openai_organization: None,
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

impl Fixture {
    fn new() -> Self {
        Self {
            settings: empty_settings(),
            azure: AzureSettings::default(),
            environment: BTreeMap::new(),
            sources: NoSources,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
            transport: Arc::new(Recorder::default()),
        }
    }

    fn context(&self) -> MessageContext<'_, Recorder> {
        MessageContext {
            settings: &self.settings,
            azure: &self.azure,
            environment: &self.environment,
            sources: &self.sources,
            transport: Arc::clone(&self.transport),
            redactor: &self.redactor,
        }
    }
}

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

fn json(body: &str) -> Json {
    serde_json::from_str(body).unwrap()
}

fn object(body: &str) -> Object {
    match json(body) {
        Json::Object(fields) => fields,
        other => panic!("not an object: {other:?}"),
    }
}

fn data(body: &str) -> MessageData {
    serde_json::from_str(body).unwrap()
}

fn params(provider: &str) -> MessageParams {
    match provider {
        "azure" => MessageParams {
            api_base: Some(String::from(AZURE_BASE)),
            api_key: Some(SecretString::from("sk-az")),
            api_version: Some(String::from("2024-05-01-preview")),
            ..MessageParams::default()
        },
        _ => MessageParams {
            api_base: Some(String::from("https://openai.test/v1")),
            api_key: Some(SecretString::from("sk-test")),
            ..MessageParams::default()
        },
    }
}

fn options(body: &str) -> MessageOptions {
    serde_json::from_str(body).unwrap()
}

fn selected(options: &MessageOptions) -> Object {
    get_optional_params_add_message(options, false).unwrap()
}

fn message_of(reply: Reply) -> ThreadMessage {
    match reply {
        Reply::Message(message) => *message,
        Reply::Page(_) => panic!("expected a message"),
    }
}

fn page_of(reply: Reply) -> ThreadMessagePage {
    match reply {
        Reply::Page(page) => *page,
        Reply::Message(_) => panic!("expected a page"),
    }
}

fn append(
    fixture: &Fixture,
    provider: &str,
    data: &MessageData,
    params: &MessageParams,
    drop_params: bool,
) -> Result<Reply, MessageError> {
    let request = MessageRequest {
        provider,
        thread_id: "thread_a",
        data: Some(data),
        params,
        drop_params,
    };
    runtime().block_on(service::add_message(&request, &fixture.context()))
}

fn append_async(
    fixture: &Fixture,
    provider: &str,
    data: &MessageData,
    params: &MessageParams,
) -> Result<Reply, MessageFailure> {
    let request = MessageRequest {
        provider,
        thread_id: "thread_a",
        data: Some(data),
        params,
        drop_params: false,
    };
    runtime().block_on(service::a_add_message(&request, &fixture.context()))
}

fn list(fixture: &Fixture, provider: &str, params: &MessageParams) -> Result<Reply, MessageError> {
    let request = MessageRequest {
        provider,
        thread_id: "thread_a",
        data: None,
        params,
        drop_params: false,
    };
    runtime().block_on(service::get_messages(&request, &fixture.context()))
}

#[test]
fn message_option_omissions() {
    assert_eq!(
        selected(&options(r#"{"provider":"openai"}"#)),
        Object::new()
    );
    assert_eq!(
        selected(&options(
            r#"{"provider":"openai","role":null,"content":null,"attachments":null,"metadata":null}"#
        )),
        Object::new()
    );
    assert_eq!(
        selected(&options(
            r#"{"provider":"openai","role":"user","content":"","attachments":[],"metadata":{}}"#
        )),
        object(r#"{"role":"user","content":"","attachments":[],"metadata":{}}"#)
    );
    for provider in ["openai", "azure", "unknown"] {
        let body =
            format!(r#"{{"provider":"{provider}","role":"user","content":"hi","trace":"x"}}"#);
        let picked = selected(&options(&body));
        assert_eq!(
            picked.get("trace"),
            Some(&json(r#""x""#)),
            "{provider} appends the extra keyword"
        );
        assert!(picked.contains_key("role"), "{provider} keeps the role");
    }
    assert_eq!(
        selected(&options(
            r#"{"provider":"anthropic","role":"user","content":"hi","trace":"x"}"#
        )),
        object(r#"{"trace":"x"}"#)
    );
}

#[test]
fn message_option_omissions_keeps_the_system_role() {
    assert_eq!(
        selected(&options(
            r#"{"provider":"openai","role":"system","content":"hi"}"#
        )),
        object(r#"{"role":"system","content":"hi"}"#)
    );
}

#[test]
fn append_openai_message() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(
        r#"{"role":"user","content":"hello","attachments":[{"file_id":"file_a","tools":[{"type":"file_search"}]}],"metadata":{"tag":"x"}}"#,
    );
    let replied =
        message_of(append(&fixture, "openai", &message, &params("openai"), false).unwrap());
    assert_eq!(replied.id, "msg_a");
    assert_eq!(replied.thread_id, "thread_a");
    assert_eq!(replied.status, Some(MessageStatus::Completed));
    let seen = fixture.transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(
        seen[0].url,
        "https://openai.test/v1/threads/thread_a/messages"
    );
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(
        seen[0].headers.get("openai-beta").map(String::as_str),
        Some("assistants=v2")
    );
    assert_eq!(
        seen[0].body,
        Some(json(
            r#"{"role":"user","content":"hello","attachments":[{"file_id":"file_a","tools":[{"type":"file_search"}]}],"metadata":{"tag":"x"}}"#
        ))
    );
}

#[test]
fn append_openai_message_async_repeats_the_request() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let sync = message_of(append(&fixture, "openai", &message, &params("openai"), false).unwrap());
    let asynchronous =
        message_of(append_async(&fixture, "openai", &message, &params("openai")).unwrap());
    assert_eq!(sync.id, asynchronous.id);
    let seen = fixture.transport.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].body, seen[1].body);
    assert_eq!(
        seen[0].body,
        Some(json(r#"{"role":"user","content":"hello"}"#))
    );
}

#[test]
fn append_openai_message_keeps_empty_lists_and_maps() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello","attachments":[],"metadata":{}}"#);
    append(&fixture, "openai", &message, &params("openai"), false).unwrap();
    assert_eq!(
        fixture.transport.requests()[0].body,
        Some(json(
            r#"{"role":"user","content":"hello","attachments":[],"metadata":{}}"#
        ))
    );
}

#[test]
fn append_openai_message_passes_content_blocks_unchanged() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":[{"type":"text","text":"hello"}]}"#);
    append(&fixture, "openai", &message, &params("openai"), false).unwrap();
    assert_eq!(
        fixture.transport.requests()[0].body,
        Some(json(
            r#"{"role":"user","content":[{"type":"text","text":"hello"}]}"#
        ))
    );
}

#[test]
fn append_openai_message_rejects_a_system_role_at_the_typed_boundary() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"system","content":"hello"}"#);
    let error = append(&fixture, "openai", &message, &params("openai"), false).unwrap_err();
    assert!(matches!(
        error,
        MessageError::Setup(SetupError::Data { .. })
    ));
    assert!(fixture.transport.requests().is_empty());
}

#[test]
fn azure_attachment_drop_and_content() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(
        r#"{"role":"user","content":"hello","attachments":[{"file_id":"file_a"}],"metadata":{"tag":"x"}}"#,
    );
    append(&fixture, "azure", &message, &params("azure"), false).unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, AZURE_URL);
    assert_eq!(
        seen[0].headers.get("api-key").map(String::as_str),
        Some("sk-az")
    );
    assert_eq!(
        seen[0].body,
        Some(json(
            r#"{"role":"user","content":"hello","metadata":{"tag":"x"}}"#
        ))
    );
}

#[test]
fn azure_attachment_drop_and_content_rejects_unsupported_attachments() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello","attachments":[{"tools":[]}]}"#);
    let error = append(&fixture, "azure", &message, &params("azure"), false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Azure doesn't support [{'tools': []}]. To drop it from the call, set `litellm.drop_params = True."
    );
    let MessageError::Setup(SetupError::Options(options)) = error else {
        panic!("expected the selector error");
    };
    assert_eq!(
        options.python_message(),
        format!("litellm.UnsupportedParamsError: {options}")
    );
    assert_eq!(options.status(), Some(400));
    assert_eq!(options.python_class(), "UnsupportedParamsError");
    assert!(fixture.transport.requests().is_empty());
}

#[test]
fn azure_attachment_drop_and_content_drops_invalid_attachments_with_drop_params() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello","attachments":[{"tools":[]}]}"#);
    append(&fixture, "azure", &message, &params("azure"), true).unwrap();
    assert_eq!(
        fixture.transport.requests()[0].body,
        Some(json(r#"{"role":"user","content":"hello"}"#))
    );
}

#[test]
fn azure_attachment_drop_and_content_rejects_non_list_attachments() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello","attachments":{}}"#);
    let error = append(&fixture, "azure", &message, &params("azure"), false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid param. attachments should always be a list. Got=<class 'dict'>, Expected=List. Raw value={}"
    );
    assert!(matches!(
        error,
        MessageError::Setup(SetupError::Options(OptionError::AttachmentsNotList { .. }))
    ));
}

#[test]
fn azure_attachment_drop_and_content_rejects_content_lists_even_with_drop_params() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":[{"type":"text","text":"hello"}]}"#);
    let error = append(&fixture, "azure", &message, &params("azure"), true).unwrap_err();
    assert_eq!(error.to_string(), "Azure only accepts content as a string.");
    assert!(matches!(
        error,
        MessageError::Setup(SetupError::Options(OptionError::ContentNotString))
    ));
    assert!(fixture.transport.requests().is_empty());
}

#[test]
fn azure_attachment_drop_and_content_maps_async_failures() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello","attachments":[{"tools":[]}]}"#);
    let failure = append_async(&fixture, "azure", &message, &params("azure")).unwrap_err();
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::MISSING);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert_eq!(
        failure.to_string(),
        "litellm.UnsupportedParamsError: Azure doesn't support [{'tools': []}]. To drop it from the call, set `litellm.drop_params = True."
    );
}

#[test]
fn message_list_ignores_pagination() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, PAGE_REPLY);
    fixture.transport.queue(200, PAGE_REPLY);
    let params: MessageParams = serde_json::from_str(
        r#"{"after":"msg_a","before":"msg_z","limit":2,"order":"asc","a_add_message":1,"aget_messages":"true","api_base":"https://openai.test/v1","api_key":"sk-test"}"#,
    )
    .unwrap();
    let sync_page = page_of(list(&fixture, "openai", &params).unwrap());
    let async_page = page_of(
        runtime()
            .block_on(service::aget_messages(
                &MessageRequest {
                    provider: "openai",
                    thread_id: "thread_a",
                    data: None,
                    params: &params,
                    drop_params: false,
                },
                &fixture.context(),
            ))
            .unwrap(),
    );
    assert!(sync_page.data.is_empty());
    assert_eq!(sync_page.has_more, Some(false));
    assert_eq!(sync_page.extra.get("object"), Some(&json(r#""list""#)));
    assert_eq!(async_page, sync_page);
    let seen = fixture.transport.requests();
    assert_eq!(seen.len(), 2);
    for request in &seen {
        assert_eq!(request.method, "GET");
        assert_eq!(
            request.url,
            "https://openai.test/v1/threads/thread_a/messages"
        );
        assert!(request.body.is_none());
    }
}

#[test]
fn message_settings_and_failures_resolves_openai_credentials() {
    let mut fixture = Fixture::new();
    fixture.settings.api_key = Some(SecretString::from("sk-global"));
    fixture.settings.organization = Some(String::from("global-org"));
    fixture.settings.openai_organization = Some(String::from("env-org"));
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let mut params = MessageParams::default();
    append(&fixture, "openai", &message, &params, false).unwrap();
    params.api_key = Some(SecretString::from("sk-request"));
    params.organization = Some(String::from("request-org"));
    append(&fixture, "openai", &message, &params, false).unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer sk-global")
    );
    assert_eq!(
        seen[0]
            .headers
            .get("openai-organization")
            .map(String::as_str),
        Some("global-org")
    );
    assert_eq!(
        seen[1].headers.get("authorization").map(String::as_str),
        Some("Bearer sk-request")
    );
    assert_eq!(
        seen[1]
            .headers
            .get("openai-organization")
            .map(String::as_str),
        Some("request-org")
    );
}

#[test]
fn message_settings_and_failures_defaults_the_deadline() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    append(&fixture, "openai", &message, &params("openai"), false).unwrap();
    assert_eq!(
        fixture.transport.requests()[0].deadline,
        Duration::from_secs(600)
    );
}

#[test]
fn message_settings_and_failures_reads_timeouts() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let mut zero = params("openai");
    zero.timeout = Some(serde_json::from_str("0").unwrap());
    zero.request_timeout = Some(30.0);
    append(&fixture, "openai", &message, &zero, false).unwrap();
    let mut text = params("openai");
    text.timeout = Some(serde_json::from_str(r#""12.5""#).unwrap());
    append(&fixture, "openai", &message, &text, false).unwrap();
    let mut structured = params("openai");
    structured.timeout = Some(serde_json::from_str(r#"{"read":5,"connect":1}"#).unwrap());
    append(&fixture, "openai", &message, &structured, false).unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(seen[0].deadline, Duration::from_secs(30));
    assert_eq!(seen[1].deadline, Duration::from_millis(12_500));
    assert_eq!(seen[2].deadline, Duration::from_secs(5));
}

fn facade_token(
    message: &MessageData,
    params: &MessageParams,
    settings: &ResourceSdkSettings,
) -> Option<String> {
    let input = PrepareInput {
        provider: "azure",
        operation: "create_thread",
        data: Some(message),
        params,
        settings,
        drop_params: false,
        asynchronous: false,
    };
    let prepared = prepare(&input).unwrap();
    let Route::Azure(credentials) = prepared.route else {
        panic!("expected the azure route");
    };
    credentials
        .facade_ad_token
        .map(|token| token.expose_secret().to_owned())
}

#[test]
fn message_settings_and_failures_selects_the_azure_ad_token() {
    let mut settings = empty_settings();
    settings.azure_ad_token = Some(SecretString::from("env-token"));
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let mut params = params("azure");
    assert_eq!(facade_token(&message, &params, &settings), None);
    params.extra_body = Some(Some(BTreeMap::new()));
    assert_eq!(facade_token(&message, &params, &settings), None);
    params.extra_body = Some(None);
    assert_eq!(
        facade_token(&message, &params, &settings),
        Some(String::from("env-token"))
    );
    params.extra_body = Some(Some(BTreeMap::from([(
        String::from("azure_ad_token"),
        json(r#""body-token""#),
    )])));
    assert_eq!(
        facade_token(&message, &params, &settings),
        Some(String::from("body-token"))
    );
}

#[test]
fn message_settings_and_failures_sends_the_azure_ad_token() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let mut params = params("azure");
    params.azure_ad_token = Some(SecretString::from("ad-token"));
    append(&fixture, "azure", &message, &params, false).unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer ad-token")
    );
    assert!(seen[0].headers.get("api-key").is_none());
}

#[test]
fn message_settings_and_failures_names_the_append_operation() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let error = append(&fixture, "anthropic", &message, &params("openai"), false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'create_thread'. Only 'openai' is supported."
    );
    let failure = append_async(&fixture, "anthropic", &message, &params("openai")).unwrap_err();
    assert!(matches!(failure.class, ErrorClass::BadRequest(_)));
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
}

#[test]
fn message_settings_and_failures_names_the_list_operation() {
    let fixture = Fixture::new();
    let error = list(&fixture, "anthropic", &params("openai")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'get_messages'. Only 'openai' is supported."
    );
    let failure = runtime()
        .block_on(service::aget_messages(
            &MessageRequest {
                provider: "anthropic",
                thread_id: "thread_a",
                data: None,
                params: &params("openai"),
                drop_params: false,
            },
            &fixture.context(),
        ))
        .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "LiteLLM doesn't support anthropic for 'get_messages'. Only 'openai' is supported."
    );
    assert!(matches!(failure.class, ErrorClass::BadRequest(_)));
}

#[test]
fn message_settings_and_failures_preserves_the_provider_body() {
    let fixture = Fixture::new();
    fixture.transport.queue(404, NOT_FOUND_BODY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let error = append(&fixture, "openai", &message, &params("openai"), false).unwrap_err();
    assert_eq!(error.status(), Some(404));
    assert_eq!(error.body(), Some(NOT_FOUND_BODY.as_bytes()));
    assert!(matches!(
        error,
        MessageError::OpenAi(ThreadsError::Status { status: 404, .. })
    ));
}

#[test]
fn message_settings_and_failures_maps_the_async_not_found() {
    let fixture = Fixture::new();
    fixture.transport.queue(404, NOT_FOUND_BODY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let failure = append_async(&fixture, "openai", &message, &params("openai")).unwrap_err();
    assert!(matches!(failure.class, ErrorClass::NotFound(_)));
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 404);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
}

#[test]
fn message_settings_and_failures_runs_the_selector_before_the_provider() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello","attachments":[{"tools":[]}]}"#);
    let error = append(&fixture, "anthropic", &message, &params("azure"), false).unwrap_err();
    assert!(matches!(
        error,
        MessageError::Setup(SetupError::Options(
            OptionError::UnsupportedAttachment { .. }
        ))
    ));
    assert!(fixture.transport.requests().is_empty());
}
