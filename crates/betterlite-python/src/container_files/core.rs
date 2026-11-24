use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::{Raw, Settings};
use betterlite_core::api::container_file_endpoints::ContainerFileEndpoints;
use betterlite_core::api::container_file_endpoints::call::DEFAULT_TIMEOUT_SECONDS;
use betterlite_core::api::container_file_list::{ContainerFileListDeps, list_container_files};
use betterlite_core::api::container_file_upload::{ContainerFileUploadDeps, upload_container_file};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::containers::container_file_http::{
    ContainerFileReply, GenericContainerHandler,
};
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::ports::{IdSource, WrapperClock};
use secrecy::SecretString;
use serde::Serialize;

use super::convert::{CallWire, ConvertError, SessionWire, parse, query_values};
use super::error::{SessionError, SessionFailure, binary, json_answer};
use super::stack;
use super::transport::Source;
use super::uploads::Uploads;
use crate::completion::setup::outbound_client;

mod asynchronous;

// The five operations the Python functions send: main.py's two and endpoint_factory.py's three
pub(crate) const LIST: &str = "list_container_files";
pub(crate) const UPLOAD: &str = "upload_container_file";
pub(crate) const RETRIEVE: &str = "retrieve_container_file";
pub(crate) const DELETE: &str = "delete_container_file";
pub(crate) const CONTENT: &str = "retrieve_container_file_content";

// LiteLLM's declared timeout default of 600 seconds, before `timeout or DEFAULT_REQUEST_TIMEOUT`
const DECLARED_TIMEOUT_SECONDS: f64 = 600.0;

// list and upload answer JSON; the content endpoint answers the provider's bytes
pub(crate) enum Answer {
    Json(String),
    Bytes(Vec<u8>),
}

pub(crate) struct SessionCore {
    settings: Settings,
    env_api_key: Option<SecretString>,
    client: ProxiedClient,
    request_timeout_seconds: f64,
    request_timeout: Duration,
    redactor: Redactor,
    decoder: stack::Decoder,
    ids: Arc<dyn IdSource>,
    clock: Arc<dyn WrapperClock>,
}

impl SessionCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let settings = Raw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        let resolved = RequestTimeout::resolve(&environment)
            .map_err(|source| SessionError::Timeout { source })?;
        let seconds = resolved.seconds;
        let request_timeout = Duration::try_from_secs_f64(seconds)
            .map_err(|source| ConvertError::Timeout { seconds, source })?;
        let env_api_key = environment
            .get("OPENAI_API_KEY")
            .filter(|key| !key.is_empty())
            .map(|key| SecretString::from(key.as_str()));
        let client = outbound_client(&environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| SessionError::Redact { source })?;
        Ok(Self {
            settings,
            env_api_key,
            client,
            request_timeout_seconds: seconds,
            request_timeout,
            redactor,
            decoder: stack::Decoder,
            ids: Arc::new(stack::SessionIds),
            clock: Arc::new(stack::SessionClock::new()),
        })
    }

    pub(crate) fn call(
        &self,
        wire: &CallWire,
        blobs: &mut [Vec<u8>],
    ) -> Result<String, SessionFailure> {
        match self.dispatch(wire, blobs)? {
            Answer::Json(text) => Ok(text),
            Answer::Bytes(_bytes) => Err(binary(wire)),
        }
    }

    pub(crate) fn content(&self, wire: &CallWire) -> Result<Vec<u8>, SessionFailure> {
        match self.dispatch(wire, &mut [])? {
            Answer::Bytes(bytes) => Ok(bytes),
            Answer::Json(_text) => Err(json_answer(wire)),
        }
    }

    pub(crate) async fn acall(
        &self,
        wire: &CallWire,
        blobs: &mut [Vec<u8>],
    ) -> Result<String, SessionFailure> {
        match asynchronous::dispatch(self, wire, blobs).await? {
            Answer::Json(text) => Ok(text),
            Answer::Bytes(_bytes) => Err(binary(wire)),
        }
    }

    pub(crate) async fn acontent(&self, wire: &CallWire) -> Result<Vec<u8>, SessionFailure> {
        match asynchronous::dispatch(self, wire, &mut []).await? {
            Answer::Bytes(bytes) => Ok(bytes),
            Answer::Json(_text) => Err(json_answer(wire)),
        }
    }

    fn dispatch(&self, wire: &CallWire, blobs: &mut [Vec<u8>]) -> Result<Answer, SessionFailure> {
        match wire.operation.as_str() {
            LIST => self.list(wire),
            UPLOAD => self.upload(wire, blobs),
            RETRIEVE => self.endpoint(wire, RETRIEVE),
            DELETE => self.endpoint(wire, DELETE),
            CONTENT => self.endpoint(wire, CONTENT),
            other => Err(SessionFailure::from(ConvertError::Operation {
                operation: other.to_owned(),
            })),
        }
    }

    fn list(&self, wire: &CallWire) -> Result<Answer, SessionFailure> {
        let arguments = &wire.arguments;
        let extra_query = query_values(arguments.extra_query.as_ref())?;
        let request = arguments.list_request(extra_query.as_ref());
        let configs = self.configs(wire);
        let logging = self.logging_state(LIST, arguments.call_id()?.as_deref())?;
        let source = Source {
            client: self.client.clone(),
            deadline: self.deadline(arguments.timeout)?,
        };
        let deps = list_deps(&self.redactor, &logging, self.request_timeout_seconds);
        let response = list_container_files(&source, &stack::NoPreCall, &configs, deps, &request)?;
        Ok(Answer::Json(encode(&response)?))
    }

    fn upload(&self, wire: &CallWire, blobs: &mut [Vec<u8>]) -> Result<Answer, SessionFailure> {
        let arguments = &wire.arguments;
        let file = arguments.upload(blobs)?;
        let extra_query = query_values(arguments.extra_query.as_ref())?;
        let request = arguments.upload_request(&file, extra_query.as_ref());
        let configs = self.configs(wire);
        let logging = self.logging_state(UPLOAD, arguments.call_id()?.as_deref())?;
        let source = Source {
            client: self.client.clone(),
            deadline: self.deadline(arguments.timeout)?,
        };
        let uploads = Uploads {
            configs: &configs,
            source,
        };
        let deps = upload_deps(&self.redactor, &logging, self.request_timeout_seconds);
        let object = upload_container_file(&uploads, deps, &request)?;
        Ok(Answer::Json(encode(&object)?))
    }

    // endpoint_factory.py's three functions: the catalog names the path, the method and the answer type
    fn endpoint(&self, wire: &CallWire, name: &str) -> Result<Answer, SessionFailure> {
        let call = wire.arguments.endpoint_call()?;
        let configs = self.configs(wire);
        let logging = self.logging_state(name, call.litellm_call_id.as_deref())?;
        let source = Source {
            client: self.client.clone(),
            deadline: self.endpoint_deadline(call.timeout),
        };
        let handler: GenericContainerHandler<Source, stack::NoPreCall> = GenericContainerHandler {
            clients: source,
            logging: stack::NoPreCall,
        };
        let endpoints =
            ContainerFileEndpoints::new(&handler, &self.decoder, &configs, &self.redactor)
                .map_err(|source| SessionError::Catalog { source })?;
        let reply =
            endpoints.container_file_call(name, &call, &logging, self.request_timeout_seconds)?;
        answer(reply)
    }

    fn logging_state(
        &self,
        call_type: &str,
        call_id: Option<&str>,
    ) -> Result<Mutex<LoggingState>, SessionFailure> {
        stack::logging_state(
            call_type,
            call_id,
            &self.redactor,
            self.ids.as_ref(),
            self.clock.as_ref(),
        )
        .map_err(SessionFailure::from)
    }

    fn configs(&self, wire: &CallWire) -> stack::Configs {
        let globals = LiteLlmGlobals {
            api_base: wire.settings.api_base.clone(),
            api_key: wire.settings.api_key.clone(),
            openai_key: wire.settings.openai_key.clone(),
            organization: wire.settings.organization.clone(),
        };
        stack::Configs::new(
            self.settings.clone(),
            globals,
            self.env_api_key.clone(),
            Arc::new(stack::NoCosts),
        )
    }

    // `timeout or DEFAULT_REQUEST_TIMEOUT`: an explicit zero takes REQUEST_TIMEOUT, an absent one takes 600
    fn deadline(&self, given: Option<f64>) -> Result<Duration, ConvertError> {
        let declared = given.unwrap_or(DECLARED_TIMEOUT_SECONDS);
        let seconds = if declared == 0.0_f64 {
            self.request_timeout_seconds
        } else {
            declared
        };
        Duration::try_from_secs_f64(seconds)
            .map_err(|source| ConvertError::Timeout { seconds, source })
    }

    // the generated functions pass `timeout or DEFAULT_REQUEST_TIMEOUT` to the handler, and it never sends it
    fn endpoint_deadline(&self, given: Option<Duration>) -> Duration {
        match given {
            Some(timeout) if !timeout.is_zero() => timeout,
            Some(_zero) | None => self.request_timeout,
        }
    }
}

pub(crate) const fn list_deps<'deps>(
    redactor: &'deps Redactor,
    logging: &'deps Mutex<LoggingState>,
    request_timeout_seconds: f64,
) -> ContainerFileListDeps<'deps> {
    ContainerFileListDeps {
        logging,
        redactor,
        request_timeout_seconds,
    }
}

pub(crate) const fn upload_deps<'deps>(
    redactor: &'deps Redactor,
    logging: &'deps Mutex<LoggingState>,
    request_timeout_seconds: f64,
) -> ContainerFileUploadDeps<'deps> {
    ContainerFileUploadDeps {
        logging,
        redactor,
        request_timeout_seconds,
    }
}

pub(crate) fn answer(reply: ContainerFileReply) -> Result<Answer, SessionFailure> {
    match reply {
        ContainerFileReply::Bytes(bytes) => Ok(Answer::Bytes(bytes)),
        ContainerFileReply::File(object) => Ok(Answer::Json(encode(&object)?)),
        ContainerFileReply::Deleted(deleted) => Ok(Answer::Json(encode(&deleted)?)),
        ContainerFileReply::List(list) => Ok(Answer::Json(encode(&list)?)),
        ContainerFileReply::Raw(value) => Ok(Answer::Json(encode(&value)?)),
    }
}

pub(crate) fn encode<T>(value: &T) -> Result<String, SessionFailure>
where
    T: Serialize,
{
    serde_json::to_string(value)
        .map_err(|source| SessionFailure::from(ConvertError::Encode { source }))
}
