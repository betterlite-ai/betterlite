#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::api::container_contracts::records::{
    ContainerFileListResponse, ContainerListResponse, ContainerObject, DeleteContainerResult,
};
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::super::error::ContainerOpFailure;
use super::super::handler::{
    ContainerCollectionHandler, CreateHandlerCall, HandlerCallBase, ListHandlerCall,
};
use super::super::{
    ContainerCallDeps, ContainerCreateCall, ContainerListCall, acreate_container, alist_containers,
    create_container, list_containers,
};
use crate::api::container_collection_http::ContainerCollectionError;
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;

pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const API_KEY: &str = "sk-test";
pub(crate) const API_BASE: &str = "https://api.openai.com/v1";
pub(crate) const API_VERSION: &str = "2024-06-01";
pub(crate) const CONTAINER_ID: &str = "cntr_original";
pub(crate) const REQUEST_TIMEOUT: f64 = 1234.0;
pub(crate) const INITIAL_CALL_TYPE: &str = "fixture_call_type";
pub(crate) const CONTAINER_JSON: &str = concat!(
    r#"{"id":"cntr_original","object":"container","created_at":3,"#,
    r#""status":"running","expires_after":null,"last_active_at":null,"name":"store"}"#,
);
pub(crate) const LIST_JSON: &str = concat!(
    r#"{"object":"list","data":[{"id":"cntr_original","object":"container","#,
    r#""created_at":3,"status":"running","expires_after":null,"#,
    r#""last_active_at":null,"name":"store"}],"first_id":"cntr_original","#,
    r#""last_id":null,"has_more":false}"#,
);

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(crate) fn logging() -> Mutex<LoggingState> {
    Mutex::new(logging_state())
}

fn logging_state() -> LoggingState {
    LoggingState::new(
        LoggingInit {
            model: "container".to_owned(),
            call_type: INITIAL_CALL_TYPE.to_owned(),
            start_time: moment(),
            litellm_call_id: CALL_ID.to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the fixture object parses")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

// the wire document is the model a mock decodes into, so the fixture cannot drift from the wire
pub(crate) fn container_object() -> ContainerObject {
    serde_json::from_str(CONTAINER_JSON).expect("the container document parses")
}

pub(crate) fn container_list() -> ContainerListResponse {
    serde_json::from_str(LIST_JSON).expect("the list document parses")
}

pub(crate) fn create_call() -> ContainerCreateCall {
    ContainerCreateCall::default()
}

pub(crate) fn list_call() -> ContainerListCall {
    ContainerListCall::default()
}

pub(crate) fn headers() -> BTreeMap<String, String> {
    BTreeMap::from([("X-Test".to_owned(), "1".to_owned())])
}

pub(crate) fn expires() -> Object {
    object(r#"{"anchor":"last_active_at","minutes":30}"#)
}

// a create call carrying every optional param, the extra headers, an extra body and a call id
pub(crate) fn full_create_call() -> ContainerCreateCall {
    let mut call = create_call();
    call.name = "store".to_owned();
    call.expires_after = Some(expires());
    call.file_ids = Some(vec!["file-1".to_owned()]);
    call.extra_headers = Some(headers());
    call.extra_body = Some(BTreeMap::from([(
        "target_model_names".to_owned(),
        "gpt-4o-mini".to_owned(),
    )]));
    call.litellm_call_id = Some(CALL_ID.to_owned());
    call
}

pub(crate) const fn declared_timeout() -> Duration {
    Duration::from_secs(600)
}

pub(crate) const fn request_timeout() -> Duration {
    Duration::from_secs(1234)
}

// the router's litellm_metadata, the first routing hint B3361's encode reads
pub(crate) fn routing_metadata(model_id: &str) -> IndexMap<String, JsonValue> {
    IndexMap::from([(
        "model_info".to_owned(),
        JsonValue::Object(IndexMap::from([(
            "id".to_owned(),
            JsonValue::Text(model_id.to_owned()),
        )])),
    )])
}

pub(crate) fn query() -> IndexMap<String, ParamValue> {
    IndexMap::from([
        ("api-version".to_owned(), ParamValue::Text("v1".to_owned())),
        ("z".to_owned(), ParamValue::Integer(2)),
    ])
}

pub(crate) fn secret(key: &str) -> SecretString {
    SecretString::from(key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HandlerForm {
    Blocking,
    Async,
}

// everything both handler calls were handed, plus the logging state as it stood when the handler ran
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SeenBase {
    pub(crate) form: HandlerForm,
    pub(crate) litellm_params: Object,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) call_type: String,
    pub(crate) provider: Option<String>,
    pub(crate) model: String,
    pub(crate) optional_params: IndexMap<String, Json>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SeenCreate {
    pub(crate) base: SeenBase,
    pub(crate) name: String,
    pub(crate) params: Object,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SeenList {
    pub(crate) base: SeenBase,
    pub(crate) after: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
    pub(crate) extra_query: Option<Object>,
}

// either operation's handler call, so the stub records both through one entry point
enum PendingCall<'call> {
    Create(&'call CreateHandlerCall<'call>),
    List(&'call ListHandlerCall<'call>),
}

fn seen_base(base: &HandlerCallBase<'_>, form: HandlerForm) -> SeenBase {
    let state = base.logging.lock().unwrap_or_else(PoisonError::into_inner);
    let seen = SeenBase {
        form,
        litellm_params: base.litellm_params.clone(),
        headers: base.extra_headers.cloned(),
        timeout: Some(base.timeout),
        call_type: state.call_type.clone(),
        provider: state.custom_llm_provider.clone(),
        model: state.model.clone(),
        optional_params: state.optional_params.clone(),
    };
    drop(state);
    seen
}

// the seam's test double: answers with its fixtures, or with the failure a test queued once
pub(crate) struct StubHandler {
    object: ContainerObject,
    list: ContainerListResponse,
    failure: Mutex<Option<ContainerCollectionError>>,
    create_seen: Mutex<Vec<SeenCreate>>,
    list_seen: Mutex<Vec<SeenList>>,
}

impl StubHandler {
    pub(crate) fn answering() -> Self {
        Self {
            object: container_object(),
            list: container_list(),
            failure: Mutex::default(),
            create_seen: Mutex::default(),
            list_seen: Mutex::default(),
        }
    }

    pub(crate) fn fail_with(&mut self, error: ContainerCollectionError) {
        self.failure = Mutex::new(Some(error));
    }

    pub(crate) fn creates(&self) -> Vec<SeenCreate> {
        let seen = self
            .create_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        seen.clone()
    }

    pub(crate) fn lists(&self) -> Vec<SeenList> {
        let seen = self
            .list_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        seen.clone()
    }

    fn answer_create(&self) -> Result<ContainerObject, ContainerCollectionError> {
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        failure.map_or_else(|| Ok(self.object.clone()), Err)
    }

    fn answer_list(&self) -> Result<ContainerListResponse, ContainerCollectionError> {
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        failure.map_or_else(|| Ok(self.list.clone()), Err)
    }

    fn record(&self, pending: &PendingCall<'_>, form: HandlerForm) {
        match pending {
            PendingCall::Create(call) => {
                let seen = SeenCreate {
                    base: seen_base(&call.base, form),
                    name: call.name.to_owned(),
                    params: call.params.clone(),
                };
                self.create_seen
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(seen);
            }
            PendingCall::List(call) => {
                self.list_seen
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(SeenList {
                        base: seen_base(&call.base, form),
                        after: call.after.map(str::to_owned),
                        limit: call.limit,
                        order: call.order.map(str::to_owned),
                        extra_query: call.extra_query.cloned(),
                    });
            }
        }
    }
}

impl ContainerCollectionHandler for StubHandler {
    fn create(
        &self,
        call: &CreateHandlerCall<'_>,
    ) -> Result<ContainerObject, ContainerCollectionError> {
        self.record(&PendingCall::Create(call), HandlerForm::Blocking);
        self.answer_create()
    }

    fn acreate(
        &self,
        call: &CreateHandlerCall<'_>,
    ) -> impl Future<Output = Result<ContainerObject, ContainerCollectionError>> + Send {
        self.record(&PendingCall::Create(call), HandlerForm::Async);
        ready(self.answer_create())
    }

    fn list(
        &self,
        call: &ListHandlerCall<'_>,
    ) -> Result<ContainerListResponse, ContainerCollectionError> {
        self.record(&PendingCall::List(call), HandlerForm::Blocking);
        self.answer_list()
    }

    fn alist(
        &self,
        call: &ListHandlerCall<'_>,
    ) -> impl Future<Output = Result<ContainerListResponse, ContainerCollectionError>> + Send {
        self.record(&PendingCall::List(call), HandlerForm::Async);
        ready(self.answer_list())
    }
}

pub(crate) struct StubConfig;

#[rustfmt::skip]
impl BaseContainerConfig for StubConfig {
    fn get_supported_openai_params(&self) -> Vec<String> { vec![String::from("stub")] }

    fn map_openai_params(&self, _params: &Object, _drop_params: bool) -> Object { Object::new() }

    fn transform_container_create_request(&self, _name: &str, _params: &Object, _context: ContainerRequestContext<'_>) -> Object { Object::new() }

    fn transform_container_create_response(&self, _raw: &[u8]) -> Result<ContainerObject, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_list_request(&self, _api_base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>) -> (String, Object) { (String::new(), Object::new()) }

    fn transform_container_list_response(&self, _raw: &[u8]) -> Result<ContainerListResponse, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_retrieve_request(&self, _id: &str, _api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_retrieve_response(&self, _raw: &[u8]) -> Result<ContainerObject, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_delete_request(&self, _id: &str, _api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_delete_response(&self, _raw: &[u8]) -> Result<DeleteContainerResult, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_file_list_request(&self, _id: &str, _api_base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>) -> Result<(String, Object), ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_file_list_response(&self, _raw: &[u8]) -> Result<ContainerFileListResponse, ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_file_content_request(&self, _id: &str, _file_id: &str, _api_base: &str, _context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { Err(ContainerError::ApiBaseRequired) }

    fn transform_container_file_content_response(&self, _raw: &[u8]) -> Result<Vec<u8>, ContainerError> { Err(ContainerError::ApiBaseRequired) }
}

// B3361's provider config manager behind the caller's registry, noting every provider asked for
pub(crate) struct StubLookup {
    config: StubConfig,
    pub(crate) missing: bool,
    asked: Mutex<Vec<ProviderId>>,
}

impl StubLookup {
    pub(crate) fn carrying() -> Self {
        Self {
            config: StubConfig,
            missing: false,
            asked: Mutex::default(),
        }
    }

    pub(crate) fn empty() -> Self {
        Self {
            config: StubConfig,
            missing: true,
            asked: Mutex::default(),
        }
    }

    pub(crate) fn asked(&self) -> Vec<ProviderId> {
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

// the logging fields the tests read, copied out so no guard is held across their assertions
pub(crate) struct LoggedState {
    pub(crate) call_type: String,
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
}

pub(crate) struct Fixture {
    pub(crate) handler: StubHandler,
    pub(crate) lookup: StubLookup,
    pub(crate) logging: Mutex<LoggingState>,
    pub(crate) redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let handler = StubHandler::answering();
        let lookup = StubLookup::carrying();
        let logging = logging();
        let redactor = redactor();
        Self {
            handler,
            lookup,
            logging,
            redactor,
        }
    }

    pub(crate) fn deps(&self) -> ContainerCallDeps<'_> {
        ContainerCallDeps {
            logging: &self.logging,
            redactor: &self.redactor,
            request_timeout_seconds: REQUEST_TIMEOUT,
        }
    }

    pub(crate) fn state(&self) -> LoggedState {
        let state = self.logging.lock().unwrap_or_else(PoisonError::into_inner);
        let call_type = state.call_type.clone();
        let model = state.model.clone();
        let custom_llm_provider = state.custom_llm_provider.clone();
        let optional_params = state.optional_params.clone();
        let litellm_params = state.litellm_params.clone();
        drop(state);
        LoggedState {
            call_type,
            model,
            custom_llm_provider,
            optional_params,
            litellm_params,
        }
    }

    pub(crate) fn create(
        &self,
        call: &ContainerCreateCall,
    ) -> Result<ContainerObject, ContainerOpFailure> {
        create_container(&self.handler, &self.lookup, self.deps(), call)
    }

    pub(crate) async fn acreate(
        &self,
        call: &ContainerCreateCall,
    ) -> Result<ContainerObject, ContainerOpFailure> {
        acreate_container(&self.handler, &self.lookup, self.deps(), call).await
    }

    pub(crate) fn list(
        &self,
        call: &ContainerListCall,
    ) -> Result<ContainerListResponse, ContainerOpFailure> {
        list_containers(&self.handler, &self.lookup, self.deps(), call)
    }

    pub(crate) async fn alist(
        &self,
        call: &ContainerListCall,
    ) -> Result<ContainerListResponse, ContainerOpFailure> {
        alist_containers(&self.handler, &self.lookup, self.deps(), call).await
    }
}
