use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;
use std::time::Duration;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_http::interactions::agents_create_read::create_async::async_create_agent;
use betterlite_http::interactions::agents_create_read::create_sync::create_agent;
use betterlite_http::interactions::agents_create_read::list_async::async_list_agents;
use betterlite_http::interactions::agents_create_read::list_sync::list_agents;
use betterlite_http::interactions::agents_create_read::logging::AgentLogging;
use betterlite_http::interactions::agents_create_read::{
    AgentRequestError, AgentsApiConfig, CreateAgentRequest, ListAgentsRequest,
};
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{ClientError, Params};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use thiserror::Error;

use super::{AgentLoggingCall, AgentsApiError, get_agents_api_config, make_logging_obj};

// LiteLLM's async entry points raise these flags in kwargs, and the sync entry points pop them before HTTP delegation
const CREATE_ASYNC_FLAG: &str = "acreate_agent";
const LIST_ASYNC_FLAG: &str = "alist_agents";
const CREATE_CALL_TYPE: &str = "create_agent";
const LIST_CALL_TYPE: &str = "list_agents";
const PROVIDER_FIELD: &str = "custom_llm_provider";
const DEFAULT_PROVIDER: &str = "gemini";

// LiteLLM's exception_type reads original_exception.message and original_exception.status_code off a provider
// error before it picks a class, so the caller's config error type reports the same two values.
pub trait AgentFailure: StdError + Send + Sync + 'static {
    fn status_code(&self) -> Option<u16>;

    fn message(&self) -> String;
}

// what the collection calls need besides the call itself: the caller's provider config, the shared HTTP clients,
// the logging state make_logging_obj updates, the logging sink the transport writes pre_call and post_call into,
// the redactor exception wrapping uses, and B198's request_timeout
pub struct AgentCollection<'deps, Config> {
    pub config: &'deps Config,
    pub clients: &'deps mut HttpClients,
    pub logging: &'deps mut LoggingState,
    pub records: &'deps mut dyn AgentLogging,
    pub redactor: &'deps Redactor,
    pub request_timeout: Duration,
}

// create's arguments after its kwargs, with base_environment kept as the caller sent it (an env id or an object)
pub struct AgentCreation<'call> {
    pub name: &'call str,
    pub base_agent: Option<&'call str>,
    pub instructions: Option<&'call str>,
    pub base_environment: Option<&'call Json>,
    pub custom_llm_provider: Option<&'call str>,
    pub extra_headers: BTreeMap<String, String>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub timeout: Option<Duration>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// list takes no name and no body: the provider, the caller's headers, a timeout and the kwargs
pub struct AgentListing<'call> {
    pub custom_llm_provider: Option<&'call str>,
    pub extra_headers: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// the sync entry answers with the result, or with the pending async call when the operation's flag asked for one
pub enum AgentCollectionHandled<Output, Pending> {
    Done(Output),
    Pending(Pending),
}

pub type AgentCollectionResult<Output, ProviderError> =
    Result<Output, AgentCollectionFailure<ProviderError>>;

#[derive(Debug, Error)]
pub enum AgentPreparationError {
    #[error(transparent)]
    Config(#[from] AgentsApiError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error("the {field} value must be a string")]
    FieldType { field: &'static str },
}

#[derive(Debug, Error)]
pub enum AgentCollectionError<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[error(transparent)]
    Prepare(#[from] AgentPreparationError),
    #[error(transparent)]
    Request(#[from] AgentRequestError<ProviderError>),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct AgentCollectionFailure<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[source]
    pub error: AgentCollectionError<ProviderError>,
    pub class: ErrorClass,
}

struct AgentCallIdentity {
    model: String,
    provider: String,
}

struct PreparedCreation {
    name: String,
    params: Params,
    extra_headers: BTreeMap<String, String>,
    extra_body: Option<Params>,
    timeout: Option<Duration>,
    is_async: bool,
}

impl PreparedCreation {
    fn request(&self, request_timeout: Duration) -> CreateAgentRequest<'_> {
        CreateAgentRequest {
            name: &self.name,
            litellm_params: &self.params,
            extra_headers: self.extra_headers.clone(),
            extra_body: self.extra_body.clone(),
            timeout: self.timeout,
            request_timeout,
        }
    }
}

struct PreparedListing {
    params: Params,
    extra_headers: BTreeMap<String, String>,
    timeout: Option<Duration>,
    is_async: bool,
}

impl PreparedListing {
    fn request(&self) -> ListAgentsRequest<'_> {
        ListAgentsRequest {
            litellm_params: &self.params,
            extra_headers: self.extra_headers.clone(),
            timeout: self.timeout,
        }
    }
}

pub fn create<'call, Config>(
    collection: AgentCollection<'call, Config>,
    call: AgentCreation<'_>,
) -> Result<
    AgentCollectionHandled<
        Config::CreateOutput,
        impl Future<Output = AgentCollectionResult<Config::CreateOutput, Config::Error>> + 'call,
    >,
    AgentCollectionFailure<Config::Error>,
>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let provider = provider_of(call.custom_llm_provider, call.kwargs).map_err(|error| {
        let named = call.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
        failure(
            AgentCollectionError::Prepare(error),
            identity_of(call.name, named),
            collection.redactor,
        )
    })?;
    let identity = identity_of(call.name, provider);
    let prepared = prepare_creation(collection.logging, &call, provider).map_err(|error| {
        failure(
            AgentCollectionError::Prepare(error),
            identity_of(call.name, provider),
            collection.redactor,
        )
    })?;
    if prepared.is_async {
        return Ok(AgentCollectionHandled::Pending(pending_creation(
            collection, prepared, identity,
        )));
    }
    finish_creation(collection, &prepared, identity).map(AgentCollectionHandled::Done)
}

pub async fn acreate<Config>(
    collection: AgentCollection<'_, Config>,
    call: AgentCreation<'_>,
) -> AgentCollectionResult<Config::CreateOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.insert(CREATE_ASYNC_FLAG.to_owned(), Json::Bool(true));
    let delegation = AgentCreation {
        name: call.name,
        base_agent: call.base_agent,
        instructions: call.instructions,
        base_environment: call.base_environment,
        custom_llm_provider: Some(call.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER)),
        extra_headers: call.extra_headers,
        extra_body: call.extra_body,
        timeout: call.timeout,
        kwargs: &kwargs,
    };
    match create(collection, delegation)? {
        AgentCollectionHandled::Done(output) => Ok(output),
        AgentCollectionHandled::Pending(pending) => pending.await,
    }
}

pub fn list<'call, Config>(
    collection: AgentCollection<'call, Config>,
    call: AgentListing<'_>,
) -> Result<
    AgentCollectionHandled<
        Config::ListOutput,
        impl Future<Output = AgentCollectionResult<Config::ListOutput, Config::Error>> + 'call,
    >,
    AgentCollectionFailure<Config::Error>,
>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let provider = provider_of(call.custom_llm_provider, call.kwargs).map_err(|error| {
        let named = call.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
        failure(
            AgentCollectionError::Prepare(error),
            identity_of("", named),
            collection.redactor,
        )
    })?;
    let identity = identity_of("", provider);
    let prepared = prepare_listing(collection.logging, &call, provider).map_err(|error| {
        failure(
            AgentCollectionError::Prepare(error),
            identity_of("", provider),
            collection.redactor,
        )
    })?;
    if prepared.is_async {
        return Ok(AgentCollectionHandled::Pending(pending_listing(
            collection, prepared, identity,
        )));
    }
    finish_listing(collection, &prepared, identity).map(AgentCollectionHandled::Done)
}

pub async fn alist<Config>(
    collection: AgentCollection<'_, Config>,
    call: AgentListing<'_>,
) -> AgentCollectionResult<Config::ListOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.insert(LIST_ASYNC_FLAG.to_owned(), Json::Bool(true));
    let delegation = AgentListing {
        custom_llm_provider: Some(call.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER)),
        extra_headers: call.extra_headers,
        timeout: call.timeout,
        kwargs: &kwargs,
    };
    match list(collection, delegation)? {
        AgentCollectionHandled::Done(output) => Ok(output),
        AgentCollectionHandled::Pending(pending) => pending.await,
    }
}

async fn pending_creation<Config>(
    collection: AgentCollection<'_, Config>,
    prepared: PreparedCreation,
    identity: AgentCallIdentity,
) -> AgentCollectionResult<Config::CreateOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let AgentCollection {
        config,
        clients,
        logging: _,
        records,
        redactor,
        request_timeout,
    } = collection;
    let request = prepared.request(request_timeout);
    async_create_agent(config, clients, records, None, &request)
        .await
        .map_err(|error| failure(AgentCollectionError::Request(error), identity, redactor))
}

fn finish_creation<Config>(
    collection: AgentCollection<'_, Config>,
    prepared: &PreparedCreation,
    identity: AgentCallIdentity,
) -> AgentCollectionResult<Config::CreateOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let AgentCollection {
        config,
        clients,
        logging: _,
        records,
        redactor,
        request_timeout,
    } = collection;
    let request = prepared.request(request_timeout);
    create_agent(config, clients, records, None, &request)
        .map_err(|error| failure(AgentCollectionError::Request(error), identity, redactor))
}

async fn pending_listing<Config>(
    collection: AgentCollection<'_, Config>,
    prepared: PreparedListing,
    identity: AgentCallIdentity,
) -> AgentCollectionResult<Config::ListOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let AgentCollection {
        config,
        clients,
        logging: _,
        records,
        redactor,
        request_timeout: _,
    } = collection;
    let request = prepared.request();
    async_list_agents(config, clients, records, None, &request)
        .await
        .map_err(|error| failure(AgentCollectionError::Request(error), identity, redactor))
}

fn finish_listing<Config>(
    collection: AgentCollection<'_, Config>,
    prepared: &PreparedListing,
    identity: AgentCallIdentity,
) -> AgentCollectionResult<Config::ListOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let AgentCollection {
        config,
        clients,
        logging: _,
        records,
        redactor,
        request_timeout: _,
    } = collection;
    let request = prepared.request();
    list_agents(config, clients, records, None, &request)
        .map_err(|error| failure(AgentCollectionError::Request(error), identity, redactor))
}

fn prepare_creation(
    logging: &mut LoggingState,
    call: &AgentCreation<'_>,
    provider: &str,
) -> Result<PreparedCreation, AgentPreparationError> {
    // LiteLLM pops the flag before it builds the params and the logging metadata
    let is_async = flag(call.kwargs, CREATE_ASYNC_FLAG);
    let kwargs = without_flag(call.kwargs, CREATE_ASYNC_FLAG);
    let mut params = params_of(&kwargs);
    insert_text(&mut params, "base_agent", call.base_agent);
    insert_text(&mut params, "instructions", call.instructions);
    if let Some(environment) = call.base_environment {
        params.insert("base_environment".to_owned(), json_value(environment));
    }
    params
        .entry(PROVIDER_FIELD.to_owned())
        .or_insert_with(|| JsonValue::Text(provider.to_owned()));
    make_logging_obj(
        logging,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: call.name,
            custom_llm_provider: provider,
            call_type: CREATE_CALL_TYPE,
            optional_params: IndexMap::new(),
        },
    )?;
    // the provider still has to have a native agents API, whatever config the caller handed in
    let _provider = get_agents_api_config(provider)?;
    Ok(PreparedCreation {
        name: call.name.to_owned(),
        params,
        extra_headers: call.extra_headers.clone(),
        extra_body: call.extra_body.map(params_of),
        timeout: call.timeout,
        is_async,
    })
}

fn prepare_listing(
    logging: &mut LoggingState,
    call: &AgentListing<'_>,
    provider: &str,
) -> Result<PreparedListing, AgentPreparationError> {
    let is_async = flag(call.kwargs, LIST_ASYNC_FLAG);
    let kwargs = without_flag(call.kwargs, LIST_ASYNC_FLAG);
    let mut params = params_of(&kwargs);
    params
        .entry(PROVIDER_FIELD.to_owned())
        .or_insert_with(|| JsonValue::Text(provider.to_owned()));
    make_logging_obj(
        logging,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: "",
            custom_llm_provider: provider,
            call_type: LIST_CALL_TYPE,
            optional_params: IndexMap::new(),
        },
    )?;
    // the provider still has to have a native agents API, whatever config the caller handed in
    let _provider = get_agents_api_config(provider)?;
    Ok(PreparedListing {
        params,
        extra_headers: call.extra_headers.clone(),
        timeout: call.timeout,
        is_async,
    })
}

// LiteLLM's `or` chain takes the default when the argument is absent, and docs/compatibility keeps an explicit
// empty string as the caller sent it instead of falling through
fn provider_of<'values>(
    argument: Option<&'values str>,
    kwargs: &'values IndexMap<String, Json>,
) -> Result<&'values str, AgentPreparationError> {
    if let Some(name) = argument {
        return Ok(name);
    }
    match kwargs.get(PROVIDER_FIELD) {
        None | Some(Json::Null) => Ok(DEFAULT_PROVIDER),
        Some(Json::Text(name)) => Ok(name),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AgentPreparationError::FieldType {
                field: PROVIDER_FIELD,
            })
        }
    }
}

// LiteLLM's kwargs.pop(name, False) is True: only an exact true starts the coroutine, and the flag never travels on
fn flag(kwargs: &IndexMap<String, Json>, name: &str) -> bool {
    matches!(kwargs.get(name), Some(Json::Bool(true)))
}

// LiteLLM's kwargs.pop(name, False): the operation's own flag never reaches the params a provider sees
fn without_flag(kwargs: &IndexMap<String, Json>, name: &str) -> IndexMap<String, Json> {
    kwargs
        .iter()
        .filter(|(field, _value)| field.as_str() != name)
        .map(|(field, value)| (field.clone(), value.clone()))
        .collect()
}

fn params_of(kwargs: &IndexMap<String, Json>) -> Params {
    kwargs
        .iter()
        .map(|(name, value)| (name.clone(), json_value(value)))
        .collect()
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), json_value(field)))
                .collect(),
        ),
    }
}

fn insert_text(params: &mut Params, name: &str, value: Option<&str>) {
    if let Some(text) = value {
        params.insert(name.to_owned(), JsonValue::Text(text.to_owned()));
    }
}

fn identity_of(model: &str, provider: &str) -> AgentCallIdentity {
    AgentCallIdentity {
        model: model.to_owned(),
        provider: provider.to_owned(),
    }
}

fn failure<ProviderError>(
    error: AgentCollectionError<ProviderError>,
    identity: AgentCallIdentity,
    redactor: &Redactor,
) -> AgentCollectionFailure<ProviderError>
where
    ProviderError: AgentFailure,
{
    let class = class_of(&error, &identity, redactor);
    AgentCollectionFailure { error, class }
}

fn class_of<ProviderError>(
    error: &AgentCollectionError<ProviderError>,
    identity: &AgentCallIdentity,
    redactor: &Redactor,
) -> ErrorClass
where
    ProviderError: AgentFailure,
{
    match error {
        AgentCollectionError::Prepare(AgentPreparationError::Config(configuration)) => {
            configuration.class()
        }
        AgentCollectionError::Prepare(AgentPreparationError::FieldType { field }) => {
            field_class(identity, *field)
        }
        AgentCollectionError::Prepare(AgentPreparationError::Logging(source)) => {
            raw_class(identity, &source.to_string(), None, redactor)
        }
        AgentCollectionError::Request(request) => request_class(request, identity, redactor),
    }
}

fn request_class<ProviderError>(
    request: &AgentRequestError<ProviderError>,
    identity: &AgentCallIdentity,
    redactor: &Redactor,
) -> ErrorClass
where
    ProviderError: AgentFailure,
{
    match request {
        AgentRequestError::Provider(source) => {
            raw_class(identity, &source.message(), source.status_code(), redactor)
        }
        AgentRequestError::Client(ClientError::FieldType { field, .. })
        | AgentRequestError::Parameters(ClientError::FieldType { field, .. }) => {
            field_class(identity, *field)
        }
        AgentRequestError::Client(source) | AgentRequestError::Parameters(source) => {
            raw_class(identity, &source.to_string(), None, redactor)
        }
        AgentRequestError::Transport(source) => {
            raw_class(identity, &source.to_string(), None, redactor)
        }
    }
}

// docs/compatibility.md rejects a value outside the declared type with a 400 naming the field
fn field_class(identity: &AgentCallIdentity, field: &'static str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
        ErrorContext {
            message: format!("the {field} value must be a string"),
            model: Some(identity.model.clone()),
            provider: Some(identity.provider.clone()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: Some(field.to_owned()),
            body_code: None,
        },
        None,
    )))
}

fn raw_class(
    identity: &AgentCallIdentity,
    text: &str,
    status_code: Option<u16>,
    redactor: &Redactor,
) -> ErrorClass {
    dispatch(
        &DispatchInput::Raw(Box::new(RawException {
            model: identity.model.clone(),
            custom_llm_provider: identity.provider.clone(),
            error_str: text.to_owned(),
            status_code,
            status_code_is_synthesized: false,
            message: Some(text.to_owned()),
            body: None,
            exception_type: String::new(),
        })),
        redactor,
    )
}

include!("collection/tests/modules.rs");
