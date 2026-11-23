use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::Section;
use betterlite_config::sections::openai_credentials::{Raw, Settings};
use betterlite_core::api::container_create_list::handler::GenericCollectionHandler;
use betterlite_core::api::container_create_list::{
    ContainerCallDeps, ContainerCreateCall, ContainerListCall, create_container, list_containers,
};
use betterlite_core::api::container_resource_ops::ContainerOperationRequest;
use betterlite_core::api::container_resource_ops::ContainerOperations;
use betterlite_core::api::container_resource_ops::handler::ContainerResourceHandler;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::container_contracts::records::ContainerObject;
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;
use serde::Serialize;

use super::super::convert::{CallWire, ConvertError, CreatedAnswer, SessionWire, parse};
use super::super::error::{SessionError, SessionFailure};
use super::super::stack;
use super::super::transport::Source;

mod asynchronous;

const CREATE_CALL_TYPE: &str = "create_container";
const LIST_CALL_TYPE: &str = "list_containers";
const RETRIEVE_CALL_TYPE: &str = "retrieve_container";
const DELETE_CALL_TYPE: &str = "delete_container";

type ResourceHandler = ContainerResourceHandler<Source, stack::NoPreCall>;
type OperationPieces = (stack::Configs, Mutex<LoggingState>, ResourceHandler);

// Everything one Python container session owns: the openai credentials, the cost map and the client every call goes through
pub(crate) struct SessionCore {
    settings: Settings,
    env_api_key: Option<SecretString>,
    costs: Arc<dyn ContainerCosts>,
    client: ProxiedClient,
    request_timeout_seconds: f64,
    redactor: Redactor,
    decoder: stack::Decoder,
}

impl SessionCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let settings = Raw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        let request_timeout = RequestTimeout::resolve(&environment)
            .map_err(|source| SessionError::Timeout { source })?;
        let env_api_key = environment
            .get("OPENAI_API_KEY")
            .filter(|key| !key.is_empty())
            .map(SecretString::from);
        let client = stack::outbound_client(&environment)?;
        let redactor = stack::redactor()?;
        let costs = Arc::new(stack::CatalogCosts::new(&environment)?);
        Ok(Self {
            settings,
            env_api_key,
            costs,
            client,
            request_timeout_seconds: request_timeout.seconds,
            redactor,
            decoder: stack::Decoder,
        })
    }

    // LiteLLM's create_container: the mock, the handler's POST, then the created record with its hidden params
    pub(crate) fn create(
        &self,
        wire: &CallWire,
        call: &ContainerCreateCall,
    ) -> Result<String, SessionFailure> {
        let configs = self.configs(wire);
        let logging = stack::logging_state(
            CREATE_CALL_TYPE,
            call.litellm_call_id.as_deref(),
            &self.redactor,
        )?;
        let source = Source {
            client: self.client.clone(),
            deadline: self.deadline(call.timeout)?,
        };
        let handler = GenericCollectionHandler {
            source,
            hook: stack::NoPreCall,
        };
        let deps = ContainerCallDeps {
            logging: &logging,
            redactor: &self.redactor,
            request_timeout_seconds: self.request_timeout_seconds,
        };
        let created = create_container(&handler, &configs, deps, call)?;
        encode_created(&created)
    }

    // LiteLLM's list_containers: the query carries only the values the caller set
    pub(crate) fn list(
        &self,
        wire: &CallWire,
        call: &ContainerListCall,
    ) -> Result<String, SessionFailure> {
        let configs = self.configs(wire);
        let logging = stack::logging_state(
            LIST_CALL_TYPE,
            call.litellm_call_id.as_deref(),
            &self.redactor,
        )?;
        let source = Source {
            client: self.client.clone(),
            deadline: self.deadline(call.timeout)?,
        };
        let handler = GenericCollectionHandler {
            source,
            hook: stack::NoPreCall,
        };
        let deps = ContainerCallDeps {
            logging: &logging,
            redactor: &self.redactor,
            request_timeout_seconds: self.request_timeout_seconds,
        };
        let reply = list_containers(&handler, &configs, deps, call)?;
        encode(&reply)
    }

    // LiteLLM's retrieve_container: the managed id decodes to its provider, then the GET reads the record back
    pub(crate) fn retrieve(
        &self,
        wire: &CallWire,
        request: &ContainerOperationRequest<'_>,
    ) -> Result<String, SessionFailure> {
        let (configs, logging, handler) =
            self.operation_stack(wire, request, RETRIEVE_CALL_TYPE)?;
        let operations = self.operations(&handler, &configs, &logging);
        let reply = operations.retrieve_container(request)?;
        encode(&reply)
    }

    pub(crate) fn delete(
        &self,
        wire: &CallWire,
        request: &ContainerOperationRequest<'_>,
    ) -> Result<String, SessionFailure> {
        let (configs, logging, handler) = self.operation_stack(wire, request, DELETE_CALL_TYPE)?;
        let operations = self.operations(&handler, &configs, &logging);
        let reply = operations.delete_container(request)?;
        encode(&reply)
    }

    // the config the call resolves against, the logging object and the handler its blocking or async form asks
    fn operation_stack(
        &self,
        wire: &CallWire,
        request: &ContainerOperationRequest<'_>,
        call_type: &str,
    ) -> Result<OperationPieces, SessionFailure> {
        let configs = self.configs(wire);
        let logging = stack::logging_state(call_type, request.litellm_call_id, &self.redactor)?;
        let source = Source {
            client: self.client.clone(),
            deadline: self.operation_deadline(request.timeout),
        };
        let handler = ContainerResourceHandler {
            clients: source,
            logging: stack::NoPreCall,
        };
        Ok((configs, logging, handler))
    }

    fn operations<'deps>(
        &'deps self,
        handler: &'deps ResourceHandler,
        configs: &'deps stack::Configs,
        logging: &'deps Mutex<LoggingState>,
    ) -> ContainerOperations<'deps, ResourceHandler, stack::Decoder, stack::Configs> {
        ContainerOperations {
            handler,
            decoder: &self.decoder,
            configs,
            logging,
            redactor: &self.redactor,
            request_timeout_seconds: self.request_timeout_seconds,
        }
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
            Arc::clone(&self.costs),
        )
    }

    // `timeout or DEFAULT_REQUEST_TIMEOUT`: an absent or zero timeout takes the configured request timeout
    fn deadline(&self, seconds: Option<f64>) -> Result<Duration, SessionFailure> {
        let seconds = match seconds {
            Some(given) if given > 0.0_f64 => given,
            Some(_) | None => self.request_timeout_seconds,
        };
        Duration::try_from_secs_f64(seconds)
            .map_err(|source| SessionFailure::from(ConvertError::Timeout { seconds, source }))
    }

    fn operation_deadline(&self, given: Option<Duration>) -> Duration {
        match given {
            Some(timeout) if !timeout.is_zero() => timeout,
            Some(_) | None => {
                Duration::try_from_secs_f64(self.request_timeout_seconds).unwrap_or(Duration::ZERO)
            }
        }
    }
}

// the created container as LiteLLM returns it: the record plus the hidden params its create transform set
fn encode_created(container: &ContainerObject) -> Result<String, SessionFailure> {
    let answer = CreatedAnswer {
        container,
        hidden_params: &container.hidden_params,
    };
    encode(&answer)
}

fn encode<T>(value: &T) -> Result<String, SessionFailure>
where
    T: Serialize,
{
    serde_json::to_string(value)
        .map_err(|source| SessionFailure::from(ConvertError::Encode { source }))
}
