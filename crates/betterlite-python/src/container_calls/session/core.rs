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
use secrecy::SecretString;
use serde::Serialize;

use super::super::convert::{
    CallWire, ConvertError, CreatedAnswer, SessionWire, parse, query_values, requested_timeout,
};
use super::super::error::{SessionError, SessionFailure};
use super::super::stack;
use super::super::transport::Source;

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

// The two collection operations as one shape: each names the logging call it makes and carries the id and timeout it logs
enum CollectionCall<'call> {
    Create(&'call ContainerCreateCall),
    List(&'call ContainerListCall),
}

impl CollectionCall<'_> {
    fn call_type(&self) -> &'static str {
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

    fn timeout(&self) -> Option<f64> {
        match self {
            Self::Create(call) => call.timeout,
            Self::List(call) => call.timeout,
        }
    }
}

// the logging call type LiteLLM's retrieve and delete name
const fn resource_call_type(delete: bool) -> &'static str {
    if delete { DELETE } else { RETRIEVE }
}

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
            .map(|key| SecretString::from(key.as_str()));
        let client = stack::outbound_client(&environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| SessionError::Redact { source })?;
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

    // the operation the request text names, through the steps LiteLLM's four functions take
    pub(crate) fn call(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        match wire.operation.as_str() {
            CREATE => self.collection(wire, CollectionCall::Create(&wire.arguments.create()?)),
            LIST => self.collection(wire, CollectionCall::List(&wire.arguments.list()?)),
            RETRIEVE => self.resource(wire, false),
            DELETE => self.resource(wire, true),
            other => Err(SessionFailure::from(ConvertError::Operation {
                operation: other.to_owned(),
            })),
        }
    }

    // LiteLLM's create_container and list_containers: the mock, the handler's call, then the encoded reply
    fn collection(
        &self,
        wire: &CallWire,
        call: CollectionCall<'_>,
    ) -> Result<String, SessionFailure> {
        let (configs, logging, handler) = self.collection_pieces(wire, &call)?;
        let deps = self.deps(&logging);
        match call {
            CollectionCall::Create(create) => {
                let created = create_container(&handler, &configs, deps, create)?;
                encode_created(&created)
            }
            CollectionCall::List(list) => encode(&list_containers(&handler, &configs, deps, list)?),
        }
    }

    // retrieve_container and delete_container share their request; only the handler method differs
    fn resource(&self, wire: &CallWire, delete: bool) -> Result<String, SessionFailure> {
        let extra_query = query_values(wire.arguments.extra_query.as_ref())?;
        let timeout = requested_timeout(wire.arguments.timeout)?;
        let request = wire
            .arguments
            .operation_request(extra_query.as_ref(), timeout);
        let (configs, logging, handler) =
            self.operation_stack(wire, &request, resource_call_type(delete))?;
        let operations = self.operations(&handler, &configs, &logging);
        match delete {
            true => encode(&operations.delete_container(&request)?),
            false => encode(&operations.retrieve_container(&request)?),
        }
    }

    // the pieces every create or list call shares, with the call type and call id its logging object takes
    fn collection_pieces(
        &self,
        wire: &CallWire,
        call: &CollectionCall<'_>,
    ) -> Result<CollectionPieces, SessionFailure> {
        let configs = self.configs(wire);
        let logging = stack::logging_state(call.call_type(), call.call_id(), &self.redactor)?;
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

    // the collaborators the collection operations read, over the logging object of this call
    fn deps<'deps>(&'deps self, logging: &'deps Mutex<LoggingState>) -> ContainerCallDeps<'deps> {
        ContainerCallDeps {
            logging,
            redactor: &self.redactor,
            request_timeout_seconds: self.request_timeout_seconds,
        }
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
