use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::{Raw, Settings};
use betterlite_core::api::container_file_endpoints::ContainerFileEndpoints;
use betterlite_core::api::container_file_endpoints::call::ContainerFileCall;
use betterlite_core::api::container_file_endpoints::call::DEFAULT_TIMEOUT_SECONDS;
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

use super::convert::{CallWire, ConvertError, SessionWire, parse};
use super::error::{SessionError, SessionFailure, binary};
use super::stack;
use super::transport::Source;
use crate::completion::setup::outbound_client;

mod tests;

const LIST: &str = "list_container_files";
const UPLOAD: &str = "upload_container_file";
const RETRIEVE: &str = "retrieve_container_file";
const DELETE: &str = "delete_container_file";
const CONTENT: &str = "retrieve_container_file_content";

type FileHandler = GenericContainerHandler<Source, stack::NoPreCall>;
type EndpointCatalog<'deps> =
    ContainerFileEndpoints<'deps, FileHandler, stack::Decoder, stack::Configs>;

// list, upload, retrieve and delete answer JSON; content answers bytes, or its mock's parsed JSON
pub(crate) enum Answer {
    Json(String),
    Bytes(Vec<u8>),
}

pub(crate) type Fallible<T> = Result<T, SessionFailure>;

pub(crate) struct SessionCore {
    settings: Settings,
    env_api_key: Option<SecretString>,
    client: ProxiedClient,
    request_timeout: Duration,
    redactor: Redactor,
    decoder: stack::Decoder,
    ids: Arc<dyn IdSource>,
    clock: Arc<dyn WrapperClock>,
}

// one call's arguments and the stack pieces the catalog call reads them through
struct Prepared {
    call: ContainerFileCall,
    configs: stack::Configs,
    logging: Mutex<LoggingState>,
    handler: FileHandler,
}

impl Prepared {
    fn advance(&self, core: &SessionCore, wire: &CallWire) -> Fallible<ContainerFileReply> {
        Ok(self.catalog(core)?.container_file_call(
            &wire.operation,
            &self.call,
            &self.logging,
            core.request_timeout.as_secs_f64(),
        )?)
    }

    fn catalog<'prepared>(
        &'prepared self,
        core: &'prepared SessionCore,
    ) -> Fallible<EndpointCatalog<'prepared>> {
        ContainerFileEndpoints::new(&self.handler, &core.decoder, &self.configs, &core.redactor)
            .map_err(|source| SessionFailure::from(SessionError::Catalog { source }))
    }
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
            request_timeout,
            redactor,
            decoder: stack::Decoder,
            ids: Arc::new(stack::SessionIds),
            clock: Arc::new(stack::SessionClock::new()),
        })
    }

    pub(crate) fn call(&self, wire: &CallWire, blobs: Vec<Vec<u8>>) -> Fallible<Answer> {
        let prepared = self.prepare(wire, blobs)?;
        let reply = prepared.advance(self, wire)?;
        answer(reply)
    }

    // the awaited path: the same catalog call, with the handler's async transport under it
    pub(crate) async fn dispatch(&self, wire: &CallWire, blobs: Vec<Vec<u8>>) -> Fallible<Answer> {
        let prepared = self.prepare(wire, blobs)?;
        let reply = prepared
            .catalog(self)?
            .acontainer_file_call(
                &wire.operation,
                &prepared.call,
                &prepared.logging,
                self.request_timeout.as_secs_f64(),
            )
            .await?;
        answer(reply)
    }

    // every operation goes through the endpoint catalog; an upload's file bytes move out first
    fn prepare(&self, wire: &CallWire, mut blobs: Vec<Vec<u8>>) -> Fallible<Prepared> {
        let file = match wire.operation.as_str() {
            LIST | RETRIEVE | DELETE | CONTENT => None,
            UPLOAD => Some(wire.arguments.upload(&mut blobs)?),
            other => {
                return Err(SessionFailure::from(ConvertError::Operation {
                    operation: other.to_owned(),
                }));
            }
        };
        let call = wire
            .arguments
            .endpoint_call(file)
            .map_err(SessionFailure::from)?;
        let logging = stack::logging_state(
            &wire.operation,
            call.litellm_call_id.as_deref(),
            &self.redactor,
            self.ids.as_ref(),
            self.clock.as_ref(),
        )
        .map_err(SessionFailure::from)?;
        // LiteLLM accepts a timeout and never sends it, so the client's own deadline applies
        let handler = GenericContainerHandler {
            clients: Source {
                client: self.client.clone(),
                deadline: Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
            },
            logging: stack::NoPreCall,
        };
        let configs = self.configs(wire);
        Ok(Prepared {
            call,
            configs,
            logging,
            handler,
        })
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
}

pub(crate) fn json_text(answer: Answer, wire: &CallWire) -> Fallible<String> {
    match answer {
        Answer::Json(text) => Ok(text),
        Answer::Bytes(_bytes) => Err(binary(wire)),
    }
}

fn answer(reply: ContainerFileReply) -> Fallible<Answer> {
    match reply {
        ContainerFileReply::Bytes(bytes) => Ok(Answer::Bytes(bytes)),
        ContainerFileReply::File(object) => Ok(Answer::Json(encode(&object)?)),
        ContainerFileReply::Deleted(deleted) => Ok(Answer::Json(encode(&deleted)?)),
        ContainerFileReply::List(list) => Ok(Answer::Json(encode(&list)?)),
        ContainerFileReply::Raw(value) => Ok(Answer::Json(encode(&value)?)),
    }
}

fn encode<T>(value: &T) -> Fallible<String>
where
    T: Serialize,
{
    serde_json::to_string(value)
        .map_err(|source| SessionFailure::from(ConvertError::Encode { source }))
}
