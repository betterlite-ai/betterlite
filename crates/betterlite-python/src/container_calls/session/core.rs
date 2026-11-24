use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::{Raw, Settings};
use betterlite_core::api::container_create_list::handler::GenericCollectionHandler;
use betterlite_core::api::container_create_list::{
    ContainerCallDeps, ContainerCreateCall, ContainerListCall, create_container, list_containers,
};
use betterlite_core::api::container_resource_http::ContainerResourceHandler;
use betterlite_core::api::container_resource_ops::ContainerOperationRequest;
use betterlite_core::api::container_resource_ops::ContainerOperations;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::container_contracts::records::ContainerObject;
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::ports::{IdSource, WrapperClock};
use secrecy::SecretString;
use serde::Serialize;

use super::super::convert::{
    CallWire, ConvertError, CreatedAnswer, SessionWire, parse, query_values, requested_timeout,
};
use super::super::error::{SessionError, SessionFailure};
use super::super::stack;
use super::super::transport::Source;
use crate::completion::setup::outbound_client;

pub(super) mod asynchronous;

mod tests;

const CREATE: &str = "create_container";
const LIST: &str = "list_containers";
const RETRIEVE: &str = "retrieve_container";
const DELETE: &str = "delete_container";

type CollectionHandler = GenericCollectionHandler<Source, stack::NoPreCall>;
type CollectionPieces = (stack::Configs, Mutex<LoggingState>, CollectionHandler);
type ResourceHandler = ContainerResourceHandler<Source, stack::NoPreCall>;
type OperationPieces = (stack::Configs, Mutex<LoggingState>, ResourceHandler);

enum CollectionCall<'call> {
    Create(&'call ContainerCreateCall),
    List(&'call ContainerListCall),
}

impl CollectionCall<'_> {
    const fn call_type(&self) -> &'static str {
        match self {
            Self::Create(_call) => CREATE,
            Self::List(_call) => LIST,
        }
    }

    fn call_id(&self) -> Option<&str> {
        match self {
            Self::Create(call) => call.litellm_call_id.as_deref(),
            Self::List(call) => call.litellm_call_id.as_deref(),
        }
    }

    const fn timeout(&self) -> Option<f64> {
        match self {
            Self::Create(call) => call.timeout,
            Self::List(call) => call.timeout,
        }
    }
}

const fn resource_call_type(delete: bool) -> &'static str {
    if delete { DELETE } else { RETRIEVE }
}

pub(crate) struct SessionCore {
    settings: Settings,
    env_api_key: Option<SecretString>,
    costs: Arc<dyn ContainerCosts>,
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
        let resolved_timeout = RequestTimeout::resolve(&environment)
            .map_err(|source| SessionError::Timeout { source })?;
        let seconds = resolved_timeout.seconds;
        let request_timeout = Duration::try_from_secs_f64(seconds)
            .map_err(|source| ConvertError::Timeout { seconds, source })?;
        let env_api_key = environment
            .get("OPENAI_API_KEY")
            .filter(|key| !key.is_empty())
            .map(|key| SecretString::from(key.as_str()));
        let client = outbound_client(&environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| SessionError::Redact { source })?;
        let costs = Arc::new(stack::CatalogCosts::new(&environment)?);
        Ok(Self {
            settings,
            env_api_key,
            costs,
            client,
            request_timeout_seconds: seconds,
            request_timeout,
            redactor,
            decoder: stack::Decoder,
            ids: Arc::new(stack::SessionIds),
            clock: Arc::new(stack::SessionClock::new()),
        })
    }

    pub(crate) fn call(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        match wire.operation.as_str() {
            CREATE => self.collection(wire, &CollectionCall::Create(&wire.arguments.create()?)),
            LIST => self.collection(wire, &CollectionCall::List(&wire.arguments.list()?)),
            RETRIEVE => self.resource(wire, false),
            DELETE => self.resource(wire, true),
            other => Err(SessionFailure::from(ConvertError::Operation {
                operation: other.to_owned(),
            })),
        }
    }

    fn collection(
        &self,
        wire: &CallWire,
        call: &CollectionCall<'_>,
    ) -> Result<String, SessionFailure> {
        let (configs, logging, handler) = self.collection_pieces(wire, call)?;
        let deps = self.deps(&logging);
        match call {
            CollectionCall::Create(create) => {
                let created = create_container(&handler, &configs, deps, create)?;
                encode_created(&created)
            }
            CollectionCall::List(list) => encode(&list_containers(&handler, &configs, deps, list)?),
        }
    }

    fn resource(&self, wire: &CallWire, delete: bool) -> Result<String, SessionFailure> {
        let extra_query = query_values(wire.arguments.extra_query.as_ref())?;
        let timeout = requested_timeout(wire.arguments.timeout)?;
        let request = wire
            .arguments
            .operation_request(extra_query.as_ref(), timeout);
        let (configs, logging, handler) =
            self.operation_stack(wire, &request, resource_call_type(delete))?;
        let operations = self.operations(&handler, &configs, &logging);
        if delete {
            encode(&operations.delete_container(&request)?)
        } else {
            encode(&operations.retrieve_container(&request)?)
        }
    }

    fn collection_pieces(
        &self,
        wire: &CallWire,
        call: &CollectionCall<'_>,
    ) -> Result<CollectionPieces, SessionFailure> {
        let configs = self.configs(wire);
        let logging = stack::logging_state(
            call.call_type(),
            call.call_id(),
            &self.redactor,
            self.ids.as_ref(),
            self.clock.as_ref(),
        )?;
        let source = Source {
            client: self.client.clone(),
            deadline: self.deadline(call.timeout())?,
        };
        let handler = GenericCollectionHandler {
            source,
            hook: stack::NoPreCall,
        };
        Ok((configs, logging, handler))
    }

    const fn deps<'deps>(&'deps self, log: &'deps Mutex<LoggingState>) -> ContainerCallDeps<'deps> {
        ContainerCallDeps {
            logging: log,
            redactor: &self.redactor,
            request_timeout_seconds: self.request_timeout_seconds,
        }
    }

    fn operation_stack(
        &self,
        wire: &CallWire,
        request: &ContainerOperationRequest<'_>,
        call_type: &str,
    ) -> Result<OperationPieces, SessionFailure> {
        let configs = self.configs(wire);
        let logging = stack::logging_state(
            call_type,
            request.litellm_call_id,
            &self.redactor,
            self.ids.as_ref(),
            self.clock.as_ref(),
        )?;
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

    const fn operations<'deps>(
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
    fn deadline(&self, requested: Option<f64>) -> Result<Duration, SessionFailure> {
        let positive = requested.filter(|given| *given > 0.0_f64);
        let Some(seconds) = positive else {
            return Ok(self.request_timeout);
        };
        Duration::try_from_secs_f64(seconds)
            .map_err(|source| SessionFailure::from(ConvertError::Timeout { seconds, source }))
    }

    const fn operation_deadline(&self, given: Option<Duration>) -> Duration {
        match given {
            Some(timeout) if !timeout.is_zero() => timeout,
            Some(_) | None => self.request_timeout,
        }
    }
}

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
