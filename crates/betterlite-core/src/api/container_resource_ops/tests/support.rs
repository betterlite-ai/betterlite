#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::api::container_contracts::records::{
    ContainerFileListResponse, ContainerListResponse, ContainerObject, DeleteContainerResult,
};
use betterlite_openai::containers::container_file_http::request::{ParamValue, SslVerify};
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource, ContainerHttpReply, ContainerHttpRequest,
    TransferError,
};
use betterlite_openai::containers::container_file_http::{ContainerFileHttpError, PreCallHook};
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::ContainerOpFailure;
use super::handler::{ContainerHandler, ContainerHandlerCall};
use super::{ContainerOperationRequest, ContainerOperations};
use crate::api::container_file_endpoints::handler::{
    ContainerConfigLookup, ContainerIdDecoder, DecodedContainerCall,
};
use crate::api::container_resource_http::{
    ContainerResourceHandler, ContainerResourceHttpError, DeleteResult, RetrieveResult,
};
use crate::containers::container_request_parameters::ContainerFieldTypeError;

pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const CONTAINER_ID: &str = "cntr_1";
pub(crate) const REQUEST_TIMEOUT: f64 = 6000.0;
pub(crate) const INITIAL_CALL_TYPE: &str = "endpoint_func";
pub(crate) const CONTAINER_JSON: &str = concat!(
    r#"{"id":"cntr_1","object":"container","created_at":1750000000,"#,
    r#""status":"running","name":"demo"}"#,
);
pub(crate) const DELETED_JSON: &str =
    r#"{"id":"cntr_1","object":"container.deleted","deleted":true}"#;

pub(crate) fn container_object() -> ContainerObject {
    serde_json::from_str(CONTAINER_JSON).expect("the container document parses")
}

pub(crate) fn params_of(text: &str) -> Object {
    serde_json::from_str(text).expect("the params fixture parses")
}

pub(crate) fn deleted_result() -> DeleteContainerResult {
    serde_json::from_str(DELETED_JSON).expect("the deleted document parses")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn logging() -> Mutex<LoggingState> {
    let init: LoggingInit = serde_json::from_str(concat!(
        r#"{"model":"container","call_type":"endpoint_func","start_time":"2026-01-01T00:00:00+00:00","#,
        r#""litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    ))
    .expect("the logging init parses");
    Mutex::new(
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
        .expect("the logging state builds"),
    )
}

pub(crate) fn provider_error(
    status_code: u16,
    message: &str,
    synthesized: bool,
) -> ContainerResourceHttpError {
    ContainerResourceHttpError::Provider {
        exception: BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers: BTreeMap::new(),
        },
        status_code_is_synthesized: synthesized,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Form {
    Blocking,
    Async,
}

// everything the handler was handed, with the logging state as it stood when the call arrived
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) container_id: String,
    pub(crate) params: Object,
    pub(crate) config_label: String,
    pub(crate) timeout: Option<Duration>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) query: Option<IndexMap<String, ParamValue>>,
    pub(crate) form: Form,
    pub(crate) call_type: String,
    pub(crate) model: String,
    pub(crate) provider: Option<String>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
}

pub(crate) struct StubHandler {
    object: Mutex<Option<ContainerObject>>,
    deleted: Mutex<Option<DeleteContainerResult>>,
    failure: Mutex<Option<ContainerResourceHttpError>>,
    seen: Mutex<Vec<Recorded>>,
}

impl StubHandler {
    fn empty() -> Self {
        Self {
            object: Mutex::default(),
            deleted: Mutex::default(),
            failure: Mutex::default(),
            seen: Mutex::default(),
        }
    }

    pub(crate) fn retrieving(object: ContainerObject) -> Self {
        Self {
            object: Mutex::new(Some(object)),
            ..Self::empty()
        }
    }

    pub(crate) fn deleting(deleted: DeleteContainerResult) -> Self {
        Self {
            deleted: Mutex::new(Some(deleted)),
            ..Self::empty()
        }
    }

    pub(crate) fn failing(error: ContainerResourceHttpError) -> Self {
        Self {
            failure: Mutex::new(Some(error)),
            ..Self::empty()
        }
    }

    pub(crate) fn seen(&self) -> Vec<Recorded> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer<T>(
        &self,
        slot: &Mutex<Option<T>>,
        missing: &str,
    ) -> Result<T, ContainerResourceHttpError> {
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(error) = failure {
            return Err(error);
        }
        let reply = slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .expect(missing);
        Ok(reply)
    }

    fn record(&self, call: &ContainerHandlerCall<'_>, form: Form) {
        let state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
        let recorded = Recorded {
            container_id: call.container_id.to_owned(),
            params: call.litellm_params.clone(),
            config_label: call
                .config
                .get_complete_url(None, call.litellm_params)
                .unwrap_or_default(),
            timeout: call.timeout,
            headers: call.extra_headers.cloned(),
            query: call.extra_query.cloned(),
            form,
            call_type: state.call_type.clone(),
            model: state.model.clone(),
            provider: state.custom_llm_provider.clone(),
            optional_params: state.optional_params.clone(),
            litellm_params: state.litellm_params.clone(),
        };
        drop(state);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
    }
}

impl ContainerHandler for StubHandler {
    fn retrieve(&self, call: &ContainerHandlerCall<'_>) -> RetrieveResult {
        self.record(call, Form::Blocking);
        self.answer(&self.object, "the fixture answers one retrieve")
    }

    fn delete(&self, call: &ContainerHandlerCall<'_>) -> DeleteResult {
        self.record(call, Form::Blocking);
        self.answer(&self.deleted, "the fixture answers one delete")
    }

    fn aretrieve<'call>(
        &'call self,
        call: &'call ContainerHandlerCall<'_>,
    ) -> impl Future<Output = RetrieveResult> + Send + 'call {
        self.record(call, Form::Async);
        ready(self.answer(&self.object, "the fixture answers one retrieve"))
    }

    fn adelete<'call>(
        &'call self,
        call: &'call ContainerHandlerCall<'_>,
    ) -> impl Future<Output = DeleteResult> + Send + 'call {
        self.record(call, Form::Async);
        ready(self.answer(&self.deleted, "the fixture answers one delete"))
    }
}

// the config a provider lookup answers, functional enough for B3365's handler to run the call
pub(crate) struct StubConfig {
    pub(crate) label: String,
}

#[rustfmt::skip]
impl BaseContainerConfig for StubConfig {
    fn get_supported_openai_params(&self) -> Vec<String> { Vec::new() }

    fn map_openai_params(&self, _params: &Object, _drop_params: bool) -> Object { Object::new() }

    fn get_complete_url(&self, api_base: Option<&str>, _litellm_params: &Object) -> Result<String, ContainerError> {
        Ok(api_base.map_or_else(|| self.label.clone(), str::to_owned))
    }

    fn transform_container_create_request(&self, _name: &str, _params: &Object, _context: ContainerRequestContext<'_>) -> Object { Object::new() }

    fn transform_container_create_response(&self, _raw: &[u8]) -> Result<ContainerObject, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_list_request(&self, _api_base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>) -> (String, Object) { (String::new(), Object::new()) }

    fn transform_container_list_response(&self, _raw: &[u8]) -> Result<ContainerListResponse, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_retrieve_request(&self, container_id: &str, api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> {
        Ok((format!("{api_base}/containers/{container_id}"), Object::new()))
    }

    fn transform_container_retrieve_response(&self, raw_response: &[u8]) -> Result<ContainerObject, ContainerError> {
        serde_json::from_slice(raw_response).map_err(|error| ContainerError::Decode(error.to_string()))
    }

    fn transform_container_delete_request(&self, container_id: &str, api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> {
        Ok((format!("{api_base}/containers/{container_id}"), Object::new()))
    }

    fn transform_container_delete_response(&self, raw_response: &[u8]) -> Result<DeleteContainerResult, ContainerError> {
        serde_json::from_slice(raw_response).map_err(|error| ContainerError::Decode(error.to_string()))
    }

    fn transform_container_file_list_request(&self, _id: &str, _api_base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>) -> Result<(String, Object), ContainerError> { Ok((String::new(), Object::new())) }

    fn transform_container_file_list_response(&self, _raw: &[u8]) -> Result<ContainerFileListResponse, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_file_content_request(&self, _id: &str, _file_id: &str, _api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { Ok((String::new(), Object::new())) }

    fn transform_container_file_content_response(&self, _raw: &[u8]) -> Result<Vec<u8>, ContainerError> { Err(ContainerError::ApiBaseRequired) }
}

pub(crate) enum StubDecode {
    Decoded {
        container_id: String,
        provider: String,
        model_id: Option<String>,
    },
    Fails,
}

pub(crate) fn decoding_to(
    container_id: &str,
    provider: &str,
    model_id: Option<&str>,
) -> StubDecode {
    StubDecode::Decoded {
        container_id: container_id.to_owned(),
        provider: provider.to_owned(),
        model_id: model_id.map(str::to_owned),
    }
}

pub(crate) struct StubDecoder {
    pub(crate) answer: StubDecode,
    seen: Mutex<Vec<(String, String, Object)>>,
}

impl StubDecoder {
    pub(crate) fn answering(answer: StubDecode) -> Self {
        Self {
            answer,
            seen: Mutex::default(),
        }
    }

    pub(crate) fn seen(&self) -> Vec<(String, String, Object)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

pub(crate) struct StubLookup {
    pub(crate) config: StubConfig,
    missing: bool,
    asked: Mutex<Vec<ProviderId>>,
}

impl StubLookup {
    pub(crate) fn carrying(label: &str) -> Self {
        Self {
            config: StubConfig {
                label: label.to_owned(),
            },
            missing: false,
            asked: Mutex::default(),
        }
    }

    pub(crate) fn without_config() -> Self {
        Self {
            config: StubConfig {
                label: String::new(),
            },
            missing: true,
            asked: Mutex::default(),
        }
    }

    pub(crate) fn asked(&self) -> Vec<ProviderId> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

// answers every request with one document and keeps what it was asked
#[derive(Clone)]
pub(crate) struct FakeClient {
    reply: ContainerHttpReply,
    seen: Arc<Mutex<Vec<ContainerHttpRequest>>>,
}

impl FakeClient {
    pub(crate) fn answering(body: &str) -> Self {
        let reply = ContainerHttpReply {
            status: 200,
            headers: BTreeMap::new(),
            body: body.as_bytes().to_vec(),
        };
        Self {
            reply,
            seen: Arc::default(),
        }
    }

    pub(crate) fn requests(&self) -> Vec<ContainerHttpRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn respond(&self, request: &ContainerHttpRequest) -> ContainerHttpReply {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        self.reply.clone()
    }
}

impl BlockingClient for FakeClient {
    fn send(&self, request: &ContainerHttpRequest) -> Result<ContainerHttpReply, TransferError> {
        Ok(self.respond(request))
    }
}

impl AsyncClient for FakeClient {
    fn send(
        &self,
        request: &ContainerHttpRequest,
    ) -> impl Future<Output = Result<ContainerHttpReply, TransferError>> + Send {
        ready(Ok(self.respond(request)))
    }
}

#[derive(Clone)]
pub(crate) struct FakeSource {
    pub(crate) client: FakeClient,
}

impl ClientSource for FakeSource {
    type Blocking = FakeClient;
    type Asynchronous = FakeClient;

    fn blocking_client(&self, _ssl_verify: Option<&SslVerify>) -> Result<FakeClient, PoolError> {
        Ok(self.client.clone())
    }

    fn async_client(&self, _ssl_verify: Option<&SslVerify>) -> Result<FakeClient, PoolError> {
        Ok(self.client.clone())
    }
}

pub(crate) struct PassingHook;

impl PreCallHook for PassingHook {
    fn pre_call(&self, _state: &mut LoggingState, _call: &PreCall) -> Result<(), PrePostError> {
        Ok(())
    }
}

pub(crate) struct Fixture {
    pub(crate) container_id: String,
    pub(crate) timeout: Option<Duration>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, ParamValue>>,
    pub(crate) extra_body: Option<IndexMap<String, JsonValue>>,
    pub(crate) litellm_call_id: Option<String>,
    pub(crate) litellm_metadata: Option<IndexMap<String, Json>>,
    pub(crate) mock_response: Option<Json>,
    pub(crate) handler: StubHandler,
    pub(crate) decoder: StubDecoder,
    pub(crate) lookup: StubLookup,
    pub(crate) logging: Mutex<LoggingState>,
    pub(crate) redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let handler = StubHandler::retrieving(container_object());
        let decoder = StubDecoder::answering(decoding_to(CONTAINER_ID, "openai", None));
        let lookup = StubLookup::carrying("openai-config");
        Self {
            container_id: CONTAINER_ID.to_owned(),
            timeout: None,
            api_key: None,
            api_base: None,
            api_version: None,
            provider: None,
            extra_headers: None,
            extra_query: None,
            extra_body: None,
            litellm_call_id: Some(CALL_ID.to_owned()),
            litellm_metadata: None,
            mock_response: None,
            handler,
            decoder,
            lookup,
            logging: logging(),
            redactor: redactor(),
        }
    }

    pub(crate) fn request(&self) -> ContainerOperationRequest<'_> {
        ContainerOperationRequest {
            container_id: &self.container_id,
            timeout: self.timeout,
            api_key: self.api_key.clone(),
            api_base: self.api_base.as_deref(),
            api_version: self.api_version.as_deref(),
            custom_llm_provider: self.provider.as_deref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_query: self.extra_query.as_ref(),
            extra_body: self.extra_body.as_ref(),
            litellm_call_id: self.litellm_call_id.as_deref(),
            litellm_metadata: self.litellm_metadata.as_ref(),
            mock_response: self.mock_response.as_ref(),
        }
    }

    pub(crate) fn ops(&self) -> ContainerOperations<'_, StubHandler, StubDecoder, StubLookup> {
        ContainerOperations {
            handler: &self.handler,
            decoder: &self.decoder,
            configs: &self.lookup,
            logging: &self.logging,
            redactor: &self.redactor,
            request_timeout_seconds: REQUEST_TIMEOUT,
        }
    }

    pub(crate) fn retrieve(&self) -> Result<ContainerObject, ContainerOpFailure> {
        let request = self.request();
        self.ops().retrieve_container(&request)
    }

    pub(crate) fn delete(&self) -> Result<DeleteContainerResult, ContainerOpFailure> {
        let request = self.request();
        self.ops().delete_container(&request)
    }

    pub(crate) async fn aretrieve(&self) -> Result<ContainerObject, ContainerOpFailure> {
        let request = self.request();
        self.ops().aretrieve_container(&request).await
    }

    pub(crate) async fn adelete(&self) -> Result<DeleteContainerResult, ContainerOpFailure> {
        let request = self.request();
        self.ops().adelete_container(&request).await
    }

    pub(crate) fn calls(&self) -> Vec<Recorded> {
        self.handler.seen()
    }
}

pub(crate) type RealHandler = ContainerResourceHandler<FakeSource, PassingHook>;

// the real B3365 handler behind the same operations, for the adapter that is not exercised by the fakes
pub(crate) fn real_ops<'deps>(
    handler: &'deps RealHandler,
    fixture: &'deps Fixture,
) -> ContainerOperations<'deps, RealHandler, StubDecoder, StubLookup> {
    ContainerOperations {
        handler,
        decoder: &fixture.decoder,
        configs: &fixture.lookup,
        logging: &fixture.logging,
        redactor: &fixture.redactor,
        request_timeout_seconds: REQUEST_TIMEOUT,
    }
}

pub(crate) fn real_handler(client: FakeClient) -> RealHandler {
    ContainerResourceHandler {
        clients: FakeSource { client },
        logging: PassingHook,
    }
}

pub(crate) fn container_refusal() -> ContainerResourceHttpError {
    ContainerResourceHttpError::Call(ContainerFileHttpError::Suspended)
}
