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
use betterlite_openai::api::container_contracts::records::ContainerFileObject;
use betterlite_openai::containers::container_file_http::ContainerFileUpload;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_prompts::media_metadata::FileData;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use super::execution::{
    AsyncUploadExecutor, ContainerFileUploadCall, UploadExecutor, UploadFailure,
};
use super::{ContainerFileUploadDeps, ContainerFileUploadRequest};
use crate::containers::container_request_parameters::ContainerProviderConfig;

pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const API_KEY: &str = "sk-test";
pub(crate) const API_BASE: &str = "https://api.openai.com/v1";
pub(crate) const API_VERSION: &str = "2024-06-01";
pub(crate) const CONTAINER_ID: &str = "cntr_original";
pub(crate) const REQUEST_TIMEOUT: f64 = 1234.0;
pub(crate) const FILE_NAME: &str = "notes.txt";
pub(crate) const FILE_BODY: &[u8] = b"hello";
pub(crate) const INITIAL_CALL_TYPE: &str = "endpoint_func";
pub(crate) const OBJECT_BODY: &str = r#"{"id": "file_1", "object": "container.file", "container_id": "cntr_remote", "bytes": 5, "created_at": 1, "path": "/notes.txt", "source": "user"}"#;

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339")
}

pub(crate) fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "gpt-4o".to_owned(),
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
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

pub(crate) fn file() -> ContainerFileUpload {
    ContainerFileUpload {
        file_data: FileData::InMemoryFile {
            name: FILE_NAME.to_owned(),
            content_type: "text/plain".to_owned(),
            content: FILE_BODY.to_vec(),
        },
    }
}

pub(crate) fn object(id: &str) -> ContainerFileObject {
    let text = OBJECT_BODY.replace("file_1", id);
    serde_json::from_str(&text).expect("the fixture file object parses")
}

pub(crate) fn kwargs() -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::new();
    kwargs.insert("litellm_call_id".to_owned(), Json::Text(CALL_ID.to_owned()));
    kwargs
}

pub(crate) struct Fixture {
    pub(crate) container_id: String,
    pub(crate) kwargs: IndexMap<String, Json>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) timeout: Option<f64>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, ParamValue>>,
    pub(crate) extra_body: Option<IndexMap<String, JsonValue>>,
    pub(crate) file: ContainerFileUpload,
    pub(crate) logging: Mutex<LoggingState>,
    pub(crate) redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self {
            container_id: CONTAINER_ID.to_owned(),
            kwargs: kwargs(),
            api_key: None,
            api_base: None,
            api_version: None,
            provider: None,
            timeout: None,
            extra_headers: None,
            extra_query: None,
            extra_body: None,
            file: file(),
            logging: Mutex::new(logging()),
            redactor: redactor(),
        }
    }

    pub(crate) fn request(&self) -> ContainerFileUploadRequest<'_> {
        ContainerFileUploadRequest {
            container_id: &self.container_id,
            file: &self.file,
            timeout: self.timeout,
            api_key: self.api_key.clone(),
            api_base: self.api_base.as_deref(),
            api_version: self.api_version.as_deref(),
            custom_llm_provider: self.provider.as_deref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_query: self.extra_query.as_ref(),
            extra_body: self.extra_body.as_ref(),
            kwargs: &self.kwargs,
        }
    }

    pub(crate) fn deps(&self) -> ContainerFileUploadDeps<'_> {
        ContainerFileUploadDeps {
            logging: &self.logging,
            redactor: &self.redactor,
            request_timeout_seconds: REQUEST_TIMEOUT,
        }
    }
}

// what the stub saw: the call's fields and the logging state the real handler would have logged with
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Seen {
    pub(crate) endpoint_name: &'static str,
    pub(crate) provider_config: ContainerProviderConfig,
    pub(crate) container_id: String,
    pub(crate) litellm_params: IndexMap<String, JsonValue>,
    pub(crate) file: ContainerFileUpload,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, ParamValue>>,
    pub(crate) timeout_seconds: f64,
    pub(crate) call_type: String,
    pub(crate) model: String,
    pub(crate) provider: Option<String>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) logging_params: IndexMap<String, Json>,
    pub(crate) details: IndexMap<String, Json>,
}

fn observe(call: &ContainerFileUploadCall<'_>) -> Seen {
    let state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
    Seen {
        endpoint_name: call.endpoint_name,
        provider_config: call.provider_config,
        container_id: call.container_id.to_owned(),
        litellm_params: call.litellm_params.clone(),
        file: call.file.clone(),
        extra_headers: call.extra_headers.cloned(),
        extra_query: call.extra_query.cloned(),
        timeout_seconds: call.timeout_seconds,
        call_type: state.call_type.clone(),
        model: state.model.clone(),
        provider: state.custom_llm_provider.clone(),
        optional_params: state.optional_params.clone(),
        logging_params: state.litellm_params.clone(),
        details: state.model_call_details.clone(),
    }
}

#[derive(Debug, Error)]
pub(crate) enum FixtureFailure {
    #[error("the provider answered {status}")]
    Provider { status: u16 },
    #[error("the connection dropped")]
    Transport,
}

impl UploadFailure for FixtureFailure {
    fn status_code(&self) -> Option<u16> {
        match self {
            Self::Provider { status } => Some(*status),
            Self::Transport => None,
        }
    }

    fn message(&self) -> String {
        self.to_string()
    }
}

enum Reply {
    Object(ContainerFileObject),
    Failure(FixtureFailure),
}

pub(crate) struct StubExecutor {
    reply: Mutex<Option<Reply>>,
    seen: Mutex<Option<Seen>>,
}

impl StubExecutor {
    pub(crate) fn replying(object: ContainerFileObject) -> Self {
        Self {
            reply: Mutex::new(Some(Reply::Object(object))),
            seen: Mutex::new(None),
        }
    }

    pub(crate) fn failing(failure: FixtureFailure) -> Self {
        Self {
            reply: Mutex::new(Some(Reply::Failure(failure))),
            seen: Mutex::new(None),
        }
    }

    pub(crate) fn seen(&self) -> Option<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn was_called(&self) -> bool {
        self.seen().is_some()
    }

    fn answer(
        &self,
        call: &ContainerFileUploadCall<'_>,
    ) -> Result<ContainerFileObject, FixtureFailure> {
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(observe(call));
        let reply = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match reply {
            Some(Reply::Object(object)) => Ok(object),
            Some(Reply::Failure(failure)) => Err(failure),
            None => panic!("the stub executor was called more than once"),
        }
    }
}

impl UploadExecutor for StubExecutor {
    type Failure = FixtureFailure;

    fn upload(
        &self,
        call: &ContainerFileUploadCall<'_>,
    ) -> Result<ContainerFileObject, Self::Failure> {
        self.answer(call)
    }
}

impl AsyncUploadExecutor for StubExecutor {
    type Failure = FixtureFailure;

    fn aupload(
        &self,
        call: &ContainerFileUploadCall<'_>,
    ) -> impl Future<Output = Result<ContainerFileObject, Self::Failure>> + Send {
        ready(self.answer(call))
    }
}
