use std::sync::{Mutex, PoisonError};

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::ContainerFileObject;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::error::{ContainerFileUploadError, ContainerFileUploadFailure, failure};
use super::execution::UploadFailure;
use super::mock::mock_response;
use super::{CALL_TYPE, Caller, ContainerFileUploadDeps, ContainerFileUploadRequest};
use crate::containers::container_request_parameters::ContainerProviderConfig;
// ast-grep-ignore: no-long-names
use crate::containers::container_request_parameters::decode_managed_container_id_for_request;
use crate::containers::container_request_parameters::get_provider_container_config;

pub(crate) enum Prepared {
    Mock(ContainerFileObject),
    Call(PreparedUpload),
}

pub(crate) struct PreparedUpload {
    pub(crate) container_id: String,
    pub(crate) provider: String,
    pub(crate) provider_config: ContainerProviderConfig,
    pub(crate) litellm_params: IndexMap<String, JsonValue>,
    pub(crate) timeout_seconds: f64,
}

// the mock short circuit returns before the provider lookup; everything after it is the real call
pub(crate) fn prepare_upload<Failure>(
    deps: &ContainerFileUploadDeps<'_>,
    request: &ContainerFileUploadRequest<'_>,
    caller: Caller<'_>,
) -> Result<Prepared, ContainerFileUploadFailure<Failure>>
where
    Failure: UploadFailure,
{
    let kwargs = call_kwargs(request);
    match mock_response(&kwargs) {
        Ok(Some(object)) => return Ok(Prepared::Mock(object)),
        Ok(None) => {}
        Err(error) => {
            return Err(failure(
                ContainerFileUploadError::Mock(error),
                caller.provider,
                deps.redactor,
            ));
        }
    }
    let litellm_params = request_params(request, &kwargs);
    let upload = resolve_call(deps, request, &kwargs, &litellm_params, caller)?;
    Ok(Prepared::Call(upload))
}

// the managed id, the provider config it resolves to, and the logging update LiteLLM does before delegating
fn resolve_call<Failure>(
    deps: &ContainerFileUploadDeps<'_>,
    request: &ContainerFileUploadRequest<'_>,
    kwargs: &IndexMap<String, Json>,
    litellm_params: &IndexMap<String, JsonValue>,
    caller: Caller<'_>,
) -> Result<PreparedUpload, ContainerFileUploadFailure<Failure>>
where
    Failure: UploadFailure,
{
    // ast-grep-ignore: no-long-names
    let decoded = decode_managed_container_id_for_request(
        request.container_id,
        caller.provider,
        litellm_params,
    )
    .map_err(|error| {
        failure(
            ContainerFileUploadError::Decode(error),
            caller.provider,
            deps.redactor,
        )
    })?;
    let provider = decoded.custom_llm_provider;
    let provider_config = get_provider_container_config(&provider).ok_or_else(|| {
        failure(
            ContainerFileUploadError::ProviderConfigMissing {
                provider: provider.clone(),
            },
            &provider,
            deps.redactor,
        )
    })?;
    log_call(deps.logging, request, kwargs, &provider).map_err(|error| {
        failure(
            ContainerFileUploadError::Logging(error),
            &provider,
            deps.redactor,
        )
    })?;
    Ok(PreparedUpload {
        container_id: decoded.container_id,
        provider,
        provider_config,
        litellm_params: decoded.litellm_params,
        timeout_seconds: timeout_seconds(caller.timeout_seconds, deps.request_timeout_seconds),
    })
}

// LiteLLM pops the async flag before GenericLiteLLMParams and the logging update see the kwargs
fn call_kwargs(request: &ContainerFileUploadRequest<'_>) -> IndexMap<String, Json> {
    let mut kwargs = request.kwargs.clone();
    kwargs.shift_remove("async_call");
    kwargs
}

// GenericLiteLLMParams(api_key=api_key, api_base=api_base, api_version=api_version, **kwargs)
fn request_params(
    request: &ContainerFileUploadRequest<'_>,
    kwargs: &IndexMap<String, Json>,
) -> IndexMap<String, JsonValue> {
    let mut params: IndexMap<String, JsonValue> = kwargs
        .iter()
        .map(|(name, value)| (name.clone(), json_value(value)))
        .collect();
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
    params
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
                .map(|(name, item)| (name.clone(), json_value(item)))
                .collect(),
        ),
    }
}

// Python's `timeout or DEFAULT_REQUEST_TIMEOUT`: the declared default is already applied, and a zero picks the request timeout
fn timeout_seconds(timeout: f64, request_timeout_seconds: f64) -> f64 {
    if timeout == 0.0_f64 {
        return request_timeout_seconds;
    }
    timeout
}

// update_from_kwargs(kwargs, model="", optional_params={"container_id": ...}, litellm_params={"litellm_call_id": ...}, custom_llm_provider=...)
fn log_call(
    logging: &Mutex<LoggingState>,
    request: &ContainerFileUploadRequest<'_>,
    kwargs: &IndexMap<String, Json>,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "container_id".to_owned(),
        Json::Text(request.container_id.to_owned()),
    );
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
            optional_params,
            additional_params,
        },
    )?;
    state.call_type = CALL_TYPE.to_owned();
    Ok(())
}
