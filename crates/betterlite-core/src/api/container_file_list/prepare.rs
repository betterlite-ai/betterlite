use std::sync::{Mutex, PoisonError};

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::api::container_contracts::records::ContainerFileListResponse;
use betterlite_openai::containers::container_file_http::transport::ClientSource;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use super::error::{ContainerFileListError, ContainerFileListFailure, failure};
use super::execution::ContainerFileListCall;
use super::mock::mock_response;
use super::{CALL_TYPE, Caller, ContainerFileListDeps, ContainerFileListRequest};
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;
use crate::containers::container_request_parameters::{
    ContainerProviderConfig, DecodedContainerRequest, get_provider_container_config,
};
// ast-grep-ignore: no-long-names
use crate::containers::container_request_parameters::decode_managed_container_id_for_request;

pub(crate) enum Prepared<'deps> {
    Mock(ContainerFileListResponse),
    Call(ReadyCall<'deps>),
}

pub(crate) struct ReadyCall<'deps> {
    pub(crate) container_id: String,
    pub(crate) provider: String,
    config: &'deps dyn BaseContainerConfig,
    litellm_params: Object,
    pub(crate) timeout_seconds: f64,
}

impl ReadyCall<'_> {
    pub(crate) fn handler_call<'call, Source: ClientSource>(
        &'call self,
        request: &'call ContainerFileListRequest<'_>,
        logging: &'call Mutex<LoggingState>,
    ) -> ContainerFileListCall<'call, Source> {
        ContainerFileListCall {
            config: self.config,
            litellm_params: &self.litellm_params,
            logging,
            extra_headers: request.extra_headers,
            extra_query: request.extra_query,
            container_id: &self.container_id,
            after: request.after,
            limit: request.limit,
            order: request.order,
            timeout_seconds: self.timeout_seconds,
            blocking_client: None,
            async_client: None,
        }
    }
}

// the mock short circuit returns before the provider lookup; everything after it is the real call
pub(crate) fn prepare_call<'deps, Configs>(
    configs: &'deps Configs,
    deps: &ContainerFileListDeps<'_>,
    request: &ContainerFileListRequest<'_>,
    caller: Caller<'_>,
) -> Result<Prepared<'deps>, ContainerFileListFailure>
where
    Configs: ContainerConfigLookup,
{
    let kwargs = call_kwargs(request);
    match mock_response(&kwargs) {
        Ok(Some(response)) => return Ok(Prepared::Mock(response)),
        Ok(None) => {}
        Err(error) => {
            return Err(failure(
                ContainerFileListError::Mock(error),
                caller.provider,
                deps.redactor,
            ));
        }
    }
    let params = request_params(request, &kwargs).map_err(|source| {
        failure(
            ContainerFileListError::Params(source),
            caller.provider,
            deps.redactor,
        )
    })?;
    let inputs = CallInputs {
        request,
        kwargs,
        params,
    };
    resolve_call(configs, deps, &inputs, caller)
}

// what the steps after the mock short circuit read: the caller's arguments, and the params they became
struct CallInputs<'request, 'arguments> {
    request: &'request ContainerFileListRequest<'arguments>,
    kwargs: IndexMap<String, Json>,
    params: IndexMap<String, JsonValue>,
}

// the managed id, the provider config it resolves to, the logging update, and the params the handler reads
fn resolve_call<'deps, Configs>(
    configs: &'deps Configs,
    deps: &ContainerFileListDeps<'_>,
    inputs: &CallInputs<'_, '_>,
    caller: Caller<'_>,
) -> Result<Prepared<'deps>, ContainerFileListFailure>
where
    Configs: ContainerConfigLookup,
{
    let decoded = decode_request(inputs.request, &inputs.params, caller, deps)?;
    let provider = decoded.custom_llm_provider;
    let config = resolve_config(configs, &provider, deps)?;
    log_call(deps.logging, inputs.request, &inputs.kwargs, &provider).map_err(|error| {
        failure(
            ContainerFileListError::Logging(error),
            &provider,
            deps.redactor,
        )
    })?;
    let litellm_params = handler_params(&decoded.litellm_params).map_err(|error| {
        failure(
            ContainerFileListError::Params(error),
            &provider,
            deps.redactor,
        )
    })?;
    Ok(Prepared::Call(ReadyCall {
        container_id: decoded.container_id,
        provider,
        config,
        litellm_params,
        timeout_seconds: timeout_seconds(caller.timeout_seconds, deps.request_timeout_seconds),
    }))
}

fn decode_request(
    request: &ContainerFileListRequest<'_>,
    params: &IndexMap<String, JsonValue>,
    caller: Caller<'_>,
    deps: &ContainerFileListDeps<'_>,
) -> Result<DecodedContainerRequest, ContainerFileListFailure> {
    // ast-grep-ignore: no-long-names
    decode_managed_container_id_for_request(request.container_id, caller.provider, params).map_err(
        |error| {
            failure(
                ContainerFileListError::Decode(error),
                caller.provider,
                deps.redactor,
            )
        },
    )
}

// B3361's provider config manager answers one enum per provider; core's caller above builds the config it names
fn resolve_config<'deps, Configs>(
    configs: &'deps Configs,
    provider: &str,
    deps: &ContainerFileListDeps<'_>,
) -> Result<&'deps dyn BaseContainerConfig, ContainerFileListFailure>
where
    Configs: ContainerConfigLookup,
{
    let kind = get_provider_container_config(provider)
        .ok_or_else(|| provider_config_missing(provider, deps))?;
    configs
        .container_config(provider_id(kind))
        .ok_or_else(|| provider_config_missing(provider, deps))
}

fn provider_config_missing(
    provider: &str,
    deps: &ContainerFileListDeps<'_>,
) -> ContainerFileListFailure {
    failure(
        ContainerFileListError::ProviderConfigMissing {
            provider: provider.to_owned(),
        },
        provider,
        deps.redactor,
    )
}

const fn provider_id(kind: ContainerProviderConfig) -> ProviderId {
    match kind {
        ContainerProviderConfig::OpenAi => ProviderId::Openai,
        ContainerProviderConfig::Azure => ProviderId::Azure,
    }
}

// LiteLLM pops the logging object and the async flag before GenericLiteLLMParams and the logging update see the kwargs
fn call_kwargs(request: &ContainerFileListRequest<'_>) -> IndexMap<String, Json> {
    let mut kwargs = request.kwargs.clone();
    kwargs.shift_remove("async_call");
    kwargs.shift_remove("litellm_logging_obj");
    kwargs
}

// GenericLiteLLMParams(api_key=api_key, api_base=api_base, api_version=api_version, **kwargs)
fn request_params(
    request: &ContainerFileListRequest<'_>,
    kwargs: &IndexMap<String, Json>,
) -> Result<IndexMap<String, JsonValue>, serde_json::Error> {
    // Json and JsonValue are the same closed JSON shape in two types, both already serialized identically, and the text keeps the caller's key order
    let encoded = serde_json::to_string(kwargs)?;
    let mut params: IndexMap<String, JsonValue> = serde_json::from_str(&encoded)?;
    let credentials = [
        (
            "api_key",
            request
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
        ),
        ("api_base", request.api_base.map(str::to_owned)),
        ("api_version", request.api_version.map(str::to_owned)),
    ];
    for (name, candidate) in credentials {
        if let Some(value) = candidate {
            params.insert(name.to_owned(), JsonValue::Text(value));
        }
    }
    Ok(params)
}

// the decoded params reach the handler as the JSON object the provider config declares
fn handler_params(params: &IndexMap<String, JsonValue>) -> Result<Object, serde_json::Error> {
    let encoded = serde_json::to_string(params)?;
    serde_json::from_str(&encoded)
}

// Python's `timeout or DEFAULT_REQUEST_TIMEOUT`: the declared default is already applied, and a zero picks the request timeout
const fn timeout_seconds(timeout: f64, request_timeout_seconds: f64) -> f64 {
    if timeout == 0.0_f64 {
        return request_timeout_seconds;
    }
    timeout
}

// update_from_kwargs(kwargs, model="", optional_params={"container_id", "after", "limit", "order"}, litellm_params={"litellm_call_id": ...}, custom_llm_provider=...)
fn log_call(
    logging: &Mutex<LoggingState>,
    request: &ContainerFileListRequest<'_>,
    kwargs: &IndexMap<String, Json>,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        request
            .kwargs
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_from_kwargs(
        &mut state,
        kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(String::new()),
            user: None,
            optional_params: logged_optional_params(request),
            additional_params,
        },
    )?;
    CALL_TYPE.clone_into(&mut state.call_type);
    drop(state);
    Ok(())
}

// LiteLLM logs all four pagination names, with None for the ones the caller left out
fn logged_optional_params(request: &ContainerFileListRequest<'_>) -> IndexMap<String, Json> {
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "container_id".to_owned(),
        Json::Text(request.container_id.to_owned()),
    );
    optional_params.insert(
        "after".to_owned(),
        request
            .after
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    optional_params.insert(
        "limit".to_owned(),
        request
            .limit
            .map_or(Json::Null, |number| Json::Number(Number::from(number))),
    );
    optional_params.insert(
        "order".to_owned(),
        request
            .order
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    optional_params
}
