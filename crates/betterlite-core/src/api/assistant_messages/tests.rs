#![cfg(test)]

mod azure;
mod openai;
mod selector;
mod settings;

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::threads::records::{ThreadMessage, ThreadMessagePage};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

use crate::api::resource_sdk_settings::ResourceSdkSettings;

use super::connection::MessageParams;
use super::error::MessageFailure;
use super::options::{MessageData, MessageOptions, select_message_params};
use super::provider::MessageContext;
use super::service::{self, MessageRequest, Operation, Reply};

const MESSAGE_REPLY: &str = r#"{"id":"msg_a","object":"thread.message","thread_id":"thread_a","role":"user","content":[],"created_at":1}"#;

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
        "azure" => serde_json::from_str(r#"{"api_base":"https://azure.test","api_key":"sk-az","api_version":"2024-05-01-preview"}"#).unwrap(),
        _ => serde_json::from_str(r#"{"api_base":"https://openai.test/v1","api_key":"sk-test"}"#).unwrap(),
    }
}

fn options(body: &str) -> MessageOptions {
    serde_json::from_str(body).unwrap()
}

fn selected(options: &MessageOptions) -> Object {
    select_message_params(options).unwrap()
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

fn request<'request>(
    provider: &'request str,
    data: Option<&'request MessageData>,
    params: &'request MessageParams,
) -> MessageRequest<'request> {
    MessageRequest {
        provider,
        thread_id: "thread_a",
        data,
        params,
        drop_params: false,
    }
}

fn call(
    fixture: &Fixture,
    request: &MessageRequest<'_>,
    operation: Operation,
    asynchronous: bool,
) -> Result<Reply, MessageFailure> {
    runtime().block_on(service::message_call(
        request,
        &fixture.context(),
        operation,
        asynchronous,
    ))
}
