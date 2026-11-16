#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Map, Value};

use crate::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use crate::api::container_contracts::error::ContainerError;
use crate::api::container_contracts::records::{
    ContainerFileListResponse, ContainerListResponse, ContainerObject, DeleteContainerResult,
};
use crate::containers::container_file_http::request::{ParamValue, SslVerify};
use crate::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource, ContainerHttpReply, ContainerHttpRequest,
    TransferError,
};
use crate::containers::container_file_http::{
    ContainerFileHttpCall, GenericContainerHandler, PreCallHook,
};

pub(super) const BASE: &str = "https://api.example.com/v1/containers";
pub(super) const FILE_BODY: &str = r#"{"id":"cfile_1","object":"container.file","container_id":"cntr_1","bytes":12,"created_at":3,"path":"/mnt/data/a.txt","source":"user"}"#;
pub(super) const LIST_BODY: &str =
    r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
pub(super) const DELETE_BODY: &str =
    r#"{"id":"cfile_1","object":"container.file.deleted","deleted":true}"#;

pub(super) type EnvironmentCall = (BTreeMap<String, String>, Option<String>);
pub(super) type BaseCall = (Option<String>, Map<String, Value>);
type Prepared = (String, Map<String, Value>);
type Reply<T> = Result<T, ContainerError>;

// Records what the handler hands the provider config, and answers with a fixed base
pub(super) struct StubConfig {
    pub(super) api_base: String,
    pub(super) environment: Mutex<Vec<EnvironmentCall>>,
    pub(super) bases: Mutex<Vec<BaseCall>>,
}

impl StubConfig {
    pub(super) fn with_base(api_base: &str) -> Self {
        Self {
            api_base: api_base.to_owned(),
            environment: Mutex::new(Vec::new()),
            bases: Mutex::new(Vec::new()),
        }
    }
}

#[rustfmt::skip]
impl BaseContainerConfig for StubConfig {
    fn get_supported_openai_params(&self) -> Vec<String> { Vec::new() }

    fn map_openai_params(&self, _params: &Map<String, Value>, _drop_params: bool) -> Map<String, Value> {
        Map::new()
    }

    fn validate_environment(&self, headers: &BTreeMap<String, String>, api_key: Option<&str>) -> BTreeMap<String, String> {
        self.environment.lock().unwrap_or_else(PoisonError::into_inner).push((headers.clone(), api_key.map(str::to_owned)));
        let mut merged = headers.clone();
        merged.insert("Authorization".to_owned(), format!("Bearer {}", api_key.unwrap_or("none")));
        merged.insert("Content-Type".to_owned(), "application/json".to_owned());
        merged.insert("content-type".to_owned(), "application/json".to_owned());
        merged
    }

    fn get_complete_url(&self, api_base: Option<&str>, litellm_params: &Map<String, Value>) -> Reply<String> {
        self.bases.lock().unwrap_or_else(PoisonError::into_inner).push((api_base.map(str::to_owned), litellm_params.clone()));
        Ok(self.api_base.clone())
    }

    fn transform_container_create_request(
        &self, _name: &str, _params: &Map<String, Value>, _context: ContainerRequestContext<'_>,
    ) -> Map<String, Value> { Map::new() }

    fn transform_container_create_response(&self, _raw: &[u8]) -> Reply<ContainerObject> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_list_request(
        &self, _base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>,
    ) -> Prepared { (String::new(), Map::new()) }

    fn transform_container_list_response(&self, _raw: &[u8]) -> Reply<ContainerListResponse> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_retrieve_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Reply<Prepared> { Ok((String::new(), Map::new())) }

    fn transform_container_retrieve_response(&self, _raw: &[u8]) -> Reply<ContainerObject> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_delete_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Reply<Prepared> { Ok((String::new(), Map::new())) }

    fn transform_container_delete_response(&self, _raw: &[u8]) -> Reply<DeleteContainerResult> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_file_list_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>,
    ) -> Reply<Prepared> { Ok((String::new(), Map::new())) }

    fn transform_container_file_list_response(&self, _raw: &[u8]) -> Reply<ContainerFileListResponse> {
        Err(ContainerError::ApiBaseRequired)
    }

    fn transform_container_file_content_request(
        &self, _id: &str, _file: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Reply<Prepared> { Ok((String::new(), Map::new())) }

    fn transform_container_file_content_response(&self, _raw: &[u8]) -> Reply<Vec<u8>> {
        Err(ContainerError::ApiBaseRequired)
    }
}

// Answers every request with one fixed reply and keeps what it was asked
#[derive(Clone)]
pub(super) struct FakeClient {
    pub(super) requests: Arc<Mutex<Vec<ContainerHttpRequest>>>,
    pub(super) reply: ContainerHttpReply,
}

impl FakeClient {
    pub(super) fn answering(status: u16, body: &[u8]) -> Self {
        let headers = BTreeMap::from([("x-request-id".to_owned(), "req_1".to_owned())]);
        let reply = ContainerHttpReply {
            status,
            headers,
            body: body.to_vec(),
        };
        Self {
            requests: Arc::default(),
            reply,
        }
    }

    pub(super) fn seen(&self) -> Vec<ContainerHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl BlockingClient for FakeClient {
    fn send(&self, request: &ContainerHttpRequest) -> Result<ContainerHttpReply, TransferError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(self.reply.clone())
    }
}

impl AsyncClient for FakeClient {
    fn send(
        &self,
        request: &ContainerHttpRequest,
    ) -> impl Future<Output = Result<ContainerHttpReply, TransferError>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        ready(Ok(self.reply.clone()))
    }
}

// Hands out clones of one client and counts how often each kind was asked for
pub(super) struct FakeSource {
    pub(super) client: FakeClient,
    pub(super) blocking_made: Mutex<Vec<Option<SslVerify>>>,
    pub(super) async_made: Mutex<Vec<Option<SslVerify>>>,
}

impl FakeSource {
    fn over(client: FakeClient) -> Self {
        Self {
            client,
            blocking_made: Mutex::default(),
            async_made: Mutex::default(),
        }
    }
}

impl ClientSource for FakeSource {
    type Blocking = FakeClient;
    type Asynchronous = FakeClient;

    fn blocking_client(&self, ssl_verify: Option<&SslVerify>) -> Result<FakeClient, PoolError> {
        self.blocking_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify.cloned());
        Ok(self.client.clone())
    }

    fn async_client(&self, ssl_verify: Option<&SslVerify>) -> Result<FakeClient, PoolError> {
        self.async_made
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify.cloned());
        Ok(self.client.clone())
    }
}

pub(super) struct RecordingHook {
    pub(super) seen: Mutex<Vec<(Json, AdditionalArgs)>>,
    pub(super) fail: bool,
}

impl RecordingHook {
    fn passing() -> Self {
        Self {
            seen: Mutex::default(),
            fail: false,
        }
    }
}

impl PreCallHook for RecordingHook {
    fn pre_call(&self, _state: &mut LoggingState, call: &PreCall) -> Result<(), PrePostError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((call.input.clone(), call.additional_args.clone()));
        if self.fail {
            return Err(PrePostError::RawRequestCaptured);
        }
        Ok(())
    }
}

pub(super) fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"container","call_type":"list_container_files","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .unwrap()
}

pub(super) type Handler = GenericContainerHandler<FakeSource, RecordingHook>;

pub(super) fn handler(status: u16, body: &str) -> Handler {
    let client = FakeClient::answering(status, body.as_bytes());
    GenericContainerHandler {
        clients: FakeSource::over(client),
        logging: RecordingHook::passing(),
    }
}

// Everything a call borrows, owned in one place so a test changes only the part it is about
pub(super) struct Fixture {
    pub(super) config: StubConfig,
    pub(super) params: Map<String, Value>,
    pub(super) logging: Mutex<LoggingState>,
    pub(super) values: BTreeMap<String, ParamValue>,
}

pub(super) fn fixture() -> Fixture {
    Fixture {
        config: StubConfig::with_base(BASE),
        params: Map::new(),
        logging: Mutex::new(logging_state()),
        values: BTreeMap::from([
            (
                "container_id".to_owned(),
                ParamValue::Text("cntr_1".to_owned()),
            ),
            ("file_id".to_owned(), ParamValue::Text("cfile_1".to_owned())),
        ]),
    }
}

impl Fixture {
    pub(super) fn call<'call>(
        &'call self,
        endpoint_name: &'call str,
    ) -> ContainerFileHttpCall<'call, FakeSource> {
        ContainerFileHttpCall {
            endpoint_name,
            config: &self.config,
            litellm_params: &self.params,
            logging: &self.logging,
            extra_headers: None,
            extra_query: None,
            timeout: None,
            values: &self.values,
            file: None,
            client: None,
        }
    }
}
