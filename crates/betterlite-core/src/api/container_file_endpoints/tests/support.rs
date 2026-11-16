#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::api::container_contracts::records::{
    ContainerFileListResponse, ContainerFileObject, ContainerListResponse, ContainerObject,
    DeleteContainerFileResponse, DeleteContainerResult,
};
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::containers::container_file_http::transport::TransferError;
use betterlite_openai::containers::container_file_http::{
    ContainerFileHttpError, ContainerFileReply, ContainerFileUpload, ReplyResult,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::io::AsyncReadExt as _;
use tokio::net::{TcpListener, TcpSocket};
use tokio::time::sleep;
use url::Url;

use super::super::ContainerFileEndpoints;
use super::super::call::ContainerFileCall;
use super::super::handler::{
    ContainerConfigLookup, ContainerFileHandler, ContainerFileHandlerCall, ContainerIdDecoder,
    DecodedContainerCall,
};
use crate::containers::container_request_parameters::ContainerFieldTypeError;

pub(super) const TIMEOUT_SECONDS: f64 = 6000.0;

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

pub(super) fn logging() -> Mutex<LoggingState> {
    Mutex::new(logging_state())
}

fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"container","call_type":"fixture_call_type","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .unwrap();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated".to_owned(),
        Duration::default,
    )
    .unwrap()
}

// the wire documents a caller passes as mock_response, as the Python models declare them
pub(super) const FILE_JSON: &str = concat!(
    r#"{"id":"cfile_1","object":"container.file","container_id":"cntr_1","bytes":12,"#,
    r#""created_at":1750000000,"path":"/mnt/data/a.txt","source":"user"}"#,
);

pub(super) const LIST_JSON: &str = concat!(
    r#"{"object":"list","data":[{"id":"cfile_1","object":"container.file","#,
    r#""container_id":"cntr_1","bytes":12,"created_at":1750000000,"#,
    r#""path":"/mnt/data/a.txt","source":"user"}],"first_id":"cfile_1","#,
    r#""last_id":"cfile_1","has_more":false}"#,
);

pub(super) const DELETED_JSON: &str =
    r#"{"id":"cfile_1","object":"container.file.deleted","deleted":true}"#;

// the same documents are the models a mock decodes into, so the fixtures cannot drift from the wire
pub(super) fn file_object() -> ContainerFileObject {
    serde_json::from_str(FILE_JSON).expect("the file document parses")
}

pub(super) fn file_list() -> ContainerFileListResponse {
    serde_json::from_str(LIST_JSON).expect("the list document parses")
}

pub(super) fn deleted_file() -> DeleteContainerFileResponse {
    serde_json::from_str(DELETED_JSON).expect("the delete document parses")
}

pub(super) fn upload() -> ContainerFileUpload {
    ContainerFileUpload {
        file_data: FileData::Direct(FileBody::Bytes(b"name,score\nalice,1\n".to_vec())),
    }
}

pub(super) fn call() -> ContainerFileCall {
    ContainerFileCall::default()
}

pub(super) fn list_call() -> ContainerFileCall {
    let mut call = call();
    call.container_id = Some("cntr_1".to_owned());
    call.after = Some("cfile_0".to_owned());
    call.limit = Some(10);
    call.order = Some("asc".to_owned());
    call
}

// a call with every typed kwarg the generated functions name
pub(super) fn credentialed_list_call() -> ContainerFileCall {
    ContainerFileCall {
        file_id: Some("cfile_7".to_owned()),
        api_key: Some(SecretString::from("sk-test")),
        api_base: Some("https://api.example".to_owned()),
        api_version: Some("2024-01-01".to_owned()),
        extra: IndexMap::from([("model".to_owned(), Json::Text("gpt-4o-mini".to_owned()))]),
        ..list_call()
    }
}

pub(super) fn managed_retrieve_call() -> ContainerFileCall {
    let mut call = retrieve_call();
    call.container_id = Some("litellm-managed-abc".to_owned());
    call
}

pub(super) fn retrieve_call() -> ContainerFileCall {
    let mut call = call();
    call.container_id = Some("cntr_1".to_owned());
    call.file_id = Some("cfile_1".to_owned());
    call
}

pub(super) fn upload_call() -> ContainerFileCall {
    let mut call = call();
    call.container_id = Some("cntr_1".to_owned());
    call.file = Some(upload());
    call
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HandlerForm {
    Blocking,
    Async,
}

// everything the handler was handed, kept so a test can read it back after the call
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Recorded {
    pub(super) endpoint_name: String,
    pub(super) config_label: String,
    pub(super) params: Object,
    pub(super) values: BTreeMap<String, ParamValue>,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) query: Option<IndexMap<String, ParamValue>>,
    pub(super) timeout: Duration,
    pub(super) file: Option<ContainerFileUpload>,
    pub(super) form: HandlerForm,
    pub(super) call_id: String,
    pub(super) optional_params: IndexMap<String, Json>,
}

pub(super) enum StubReply {
    List(ContainerFileListResponse),
    File(ContainerFileObject),
    Deleted(DeleteContainerFileResponse),
    Bytes(Vec<u8>),
    Status { status_code: u16, message: String },
    ConfigStatus { status_code: u16, message: String },
    Suspended,
}

impl StubReply {
    fn build(&self) -> ReplyResult {
        match self {
            Self::List(list) => Ok(ContainerFileReply::List(list.clone())),
            Self::File(file) => Ok(ContainerFileReply::File(file.clone())),
            Self::Deleted(deleted) => Ok(ContainerFileReply::Deleted(deleted.clone())),
            Self::Bytes(bytes) => Ok(ContainerFileReply::Bytes(bytes.clone())),
            Self::Status {
                status_code,
                message,
            } => Err(ContainerFileHttpError::Provider(BaseLlmException {
                status_code: *status_code,
                message: message.clone(),
                headers: BTreeMap::new(),
            })),
            Self::ConfigStatus {
                status_code,
                message,
            } => Err(ContainerFileHttpError::Config(ContainerError::Upstream(
                BaseLlmException {
                    status_code: *status_code,
                    message: message.clone(),
                    headers: BTreeMap::new(),
                },
            ))),
            Self::Suspended => Err(ContainerFileHttpError::Suspended),
        }
    }
}

const LOCAL_TIMEOUT: Duration = Duration::from_millis(100);
const LOCAL_HOLD: Duration = Duration::from_millis(300);

fn local_client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    ProxiedClient::build(base_builder(), &policy, limits).expect("the client")
}

// the client's own read timeout is the only source of a Deadline, so a local listener keeps the answer back
pub(super) async fn deadline_failure() -> ContainerFileHttpError {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    let client = local_client(TransportLimits {
        read_timeout: LOCAL_TIMEOUT,
        ..TransportLimits::DEFAULT
    });
    let url = Url::parse(&format!("http://{address}/v1/files")).expect("the URL");
    let held = async {
        let (mut stream, _) = listener.accept().await.expect("the connection");
        let mut chunk = [0_u8; 64];
        let read = stream.read(&mut chunk).await.expect("the request bytes");
        assert_ne!(read, 0, "the client sends its request");
        sleep(LOCAL_HOLD).await;
    };
    let (result, ()) = tokio::join!(client.get(url, HeaderMap::new()), held);
    let error = result.expect_err("the read timeout ends the request");
    ContainerFileHttpError::Transfer(TransferError::Transport(error))
}

// a listener whose accept queue is full, so a connect waits until the client's own connect timeout
pub(super) async fn connect_timeout_failure() -> ContainerFileHttpError {
    let socket = TcpSocket::new_v4().expect("the socket");
    socket
        .bind("127.0.0.1:0".parse::<SocketAddr>().expect("the address"))
        .expect("the bind");
    let listener = socket.listen(1).expect("the listen");
    let address = listener.local_addr().expect("the address");
    let _fillers: Vec<TcpStream> = (0_u8..4)
        .filter_map(|_| TcpStream::connect_timeout(&address, LOCAL_TIMEOUT).ok())
        .collect();
    let client = local_client(TransportLimits {
        connect_timeout: LOCAL_TIMEOUT,
        ..TransportLimits::DEFAULT
    });
    let url = Url::parse(&format!("http://{address}/v1/files")).expect("the URL");
    let error = client
        .get(url, HeaderMap::new())
        .await
        .expect_err("the connect times out");
    ContainerFileHttpError::Transfer(TransferError::Transport(error))
}

pub(super) struct StubHandler {
    pub(super) reply: StubReply,
    failure: Mutex<Option<ContainerFileHttpError>>,
    seen: Mutex<Vec<Recorded>>,
}

impl StubHandler {
    pub(super) fn answering(reply: StubReply) -> Self {
        Self {
            reply,
            failure: Mutex::default(),
            seen: Mutex::default(),
        }
    }

    pub(super) fn fail_with(&mut self, error: ContainerFileHttpError) {
        self.failure = Mutex::new(Some(error));
    }

    pub(super) fn seen(&self) -> Vec<Recorded> {
        let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.clone()
    }

    fn answer(&self) -> ReplyResult {
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match failure {
            Some(error) => Err(error),
            None => self.reply.build(),
        }
    }

    fn record(&self, call: &ContainerFileHandlerCall<'_>, form: HandlerForm) {
        let state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
        let recorded = Recorded {
            endpoint_name: call.endpoint_name.to_owned(),
            config_label: call
                .config
                .get_complete_url(None, call.params)
                .unwrap_or_default(),
            params: call.params.clone(),
            values: call.values.clone(),
            headers: call.extra_headers.cloned(),
            query: call.extra_query.cloned(),
            timeout: call.timeout,
            file: call.file.cloned(),
            form,
            call_id: state.litellm_call_id.clone(),
            optional_params: state.optional_params.clone(),
        };
        drop(state);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
    }
}

impl ContainerFileHandler for StubHandler {
    fn handle(&self, call: &ContainerFileHandlerCall<'_>) -> ReplyResult {
        self.record(call, HandlerForm::Blocking);
        self.answer()
    }

    fn ahandle(
        &self,
        call: &ContainerFileHandlerCall<'_>,
    ) -> impl Future<Output = ReplyResult> + Send {
        self.record(call, HandlerForm::Async);
        ready(self.answer())
    }
}

pub(super) struct StubConfig {
    pub(super) label: String,
}

#[rustfmt::skip]
impl BaseContainerConfig for StubConfig {
    fn get_supported_openai_params(&self) -> Vec<String> { vec![self.label.clone()] }

    fn map_openai_params(&self, _params: &Object, _drop_params: bool) -> Object { Object::new() }

    fn get_complete_url(&self, _api_base: Option<&str>, _litellm_params: &Object) -> Result<String, ContainerError> {
        Ok(self.label.clone())
    }

    fn transform_container_create_request(&self, _name: &str, _params: &Object, _context: ContainerRequestContext<'_>) -> Object { Object::new() }

    fn transform_container_create_response(&self, _raw: &[u8]) -> Result<ContainerObject, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_list_request(&self, _api_base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>) -> (String, Object) { (String::new(), Object::new()) }

    fn transform_container_list_response(&self, _raw: &[u8]) -> Result<ContainerListResponse, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_retrieve_request(&self, _id: &str, _api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { Ok((String::new(), Object::new())) }

    fn transform_container_retrieve_response(&self, _raw: &[u8]) -> Result<ContainerObject, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_delete_request(&self, _id: &str, _api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { Ok((String::new(), Object::new())) }

    fn transform_container_delete_response(&self, _raw: &[u8]) -> Result<DeleteContainerResult, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_file_list_request(&self, _id: &str, _api_base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>) -> Result<(String, Object), ContainerError> { Ok((String::new(), Object::new())) }

    fn transform_container_file_list_response(&self, _raw: &[u8]) -> Result<ContainerFileListResponse, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_file_content_request(&self, _id: &str, _file_id: &str, _api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { Ok((String::new(), Object::new())) }

    fn transform_container_file_content_response(&self, _raw: &[u8]) -> Result<Vec<u8>, ContainerError> { Err(ContainerError::ApiBaseRequired) }
}

pub(super) enum StubDecode {
    Decoded {
        container_id: String,
        provider: String,
        model_id: Option<String>,
    },
    Fails,
}

pub(super) fn decoding_to(provider: &str, model_id: Option<&str>) -> StubDecode {
    StubDecode::Decoded {
        container_id: "cntr_original".to_owned(),
        provider: provider.to_owned(),
        model_id: model_id.map(str::to_owned),
    }
}

pub(super) struct StubDecoder {
    pub(super) answer: StubDecode,
    seen: Mutex<Vec<(String, String, Object)>>,
}

impl StubDecoder {
    pub(super) fn answering(answer: StubDecode) -> Self {
        Self {
            answer,
            seen: Mutex::default(),
        }
    }

    pub(super) fn seen(&self) -> Vec<(String, String, Object)> {
        let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.clone()
    }
}

impl ContainerIdDecoder for StubDecoder {
    fn decode(
        &self,
        container_id: &str,
        provider: &str,
        params: &Object,
    ) -> Result<DecodedContainerCall, ContainerFieldTypeError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((container_id.to_owned(), provider.to_owned(), params.clone()));
        match &self.answer {
            StubDecode::Decoded {
                container_id: decoded_id,
                provider: decoded_provider,
                model_id,
            } => {
                let mut litellm_params = params.clone();
                if let Some(deployment) = model_id {
                    litellm_params.insert("model_id".to_owned(), serde_json::json!(deployment));
                }
                Ok(DecodedContainerCall {
                    container_id: decoded_id.clone(),
                    custom_llm_provider: decoded_provider.clone(),
                    litellm_params,
                })
            }
            StubDecode::Fails => Err(ContainerFieldTypeError { field: "model_id" }),
        }
    }
}

pub(super) struct StubLookup {
    pub(super) config: StubConfig,
    pub(super) missing: bool,
    asked: Mutex<Vec<ProviderId>>,
}

impl StubLookup {
    pub(super) fn carrying(label: &str) -> Self {
        Self {
            config: StubConfig {
                label: label.to_owned(),
            },
            missing: false,
            asked: Mutex::default(),
        }
    }

    pub(super) fn without_config() -> Self {
        Self {
            config: StubConfig {
                label: String::new(),
            },
            missing: true,
            asked: Mutex::default(),
        }
    }

    pub(super) fn asked(&self) -> Vec<ProviderId> {
        let asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        asked.clone()
    }
}

impl ContainerConfigLookup for StubLookup {
    fn container_config(&self, provider: ProviderId) -> Option<&dyn BaseContainerConfig> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(provider);
        if self.missing {
            return None;
        }
        Some(&self.config)
    }
}

pub(super) struct Fixture {
    pub(super) handler: StubHandler,
    pub(super) decoder: StubDecoder,
    pub(super) lookup: StubLookup,
    pub(super) redactor: Redactor,
}

pub(super) fn fixture() -> Fixture {
    Fixture {
        handler: StubHandler::answering(StubReply::File(file_object())),
        decoder: StubDecoder::answering(StubDecode::Decoded {
            container_id: "cntr_1".to_owned(),
            provider: "openai".to_owned(),
            model_id: None,
        }),
        lookup: StubLookup::carrying("openai"),
        redactor: redactor(),
    }
}

impl Fixture {
    pub(super) fn endpoints(
        &self,
    ) -> ContainerFileEndpoints<'_, StubHandler, StubDecoder, StubLookup> {
        ContainerFileEndpoints::new(&self.handler, &self.decoder, &self.lookup, &self.redactor)
            .expect("the catalog parses")
    }
}
