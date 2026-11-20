use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::api::container_contracts::records::ContainerObject;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::ExposeSecret;

use super::super::error::{ContainerOpError, ContainerOpFailure, failure};
use super::super::handler::{CreateHandlerCall, HandlerCallBase};
use super::super::logging::{LoggedCall, log_call};
use super::super::params;
use super::super::{CREATE_CALL_TYPE, ContainerCallDeps, ContainerCreateCall};
use super::{ContainerOperation, mock_answer, provider_config, resolved_timeout};
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;
use crate::containers::container_request_parameters::{
    // ast-grep-ignore: no-long-names
    get_optional_params_container_create,
    // ast-grep-ignore: no-long-names
    get_requested_container_create_optional_param,
};

// a mock answer, or everything the create handler call needs
pub(crate) enum PreparedCreate<'deps> {
    Mock(ContainerObject),
    Call(ReadyCreate<'deps>),
}

pub(crate) struct ReadyCreate<'deps> {
    config: &'deps dyn BaseContainerConfig,
    litellm_params: Object,
    params: Object,
    timeout: Duration,
}

impl ReadyCreate<'_> {
    pub(crate) const fn handler_call<'call>(
        &'call self,
        call: &'call ContainerCreateCall,
        logging: &'call Mutex<LoggingState>,
    ) -> CreateHandlerCall<'call> {
        CreateHandlerCall {
            base: HandlerCallBase {
                config: self.config,
                litellm_params: &self.litellm_params,
                logging,
                extra_headers: call.extra_headers.as_ref(),
                timeout: Some(self.timeout),
            },
            name: &call.name,
            params: &self.params,
        }
    }
}

// create_container's steps 1 to 7, shared by the blocking and async operations
pub(crate) fn prepare_create<'deps, Configs>(
    configs: &'deps Configs,
    deps: &ContainerCallDeps<'_>,
    call: &ContainerCreateCall,
) -> Result<PreparedCreate<'deps>, ContainerOpFailure>
where
    Configs: ContainerConfigLookup,
{
    let mock = mock_answer(
        call.mock_response.as_ref(),
        "a container object",
        &call.custom_llm_provider,
        deps.redactor,
    )?;
    if let Some(container) = mock {
        return Ok(PreparedCreate::Mock(container));
    }
    build_create(configs, deps, call)
        .map_err(|error| failure(error, &call.custom_llm_provider, deps.redactor))
}

// steps 2 to 7, in LiteLLM's order, each returning this module's error for the one mapping above
fn build_create<'deps, Configs>(
    configs: &'deps Configs,
    deps: &ContainerCallDeps<'_>,
    call: &ContainerCreateCall,
) -> Result<PreparedCreate<'deps>, ContainerOpError>
where
    Configs: ContainerConfigLookup,
{
    let (kwargs, litellm_params) = call_parameters(call)?;
    let config = provider_config(
        configs,
        &call.custom_llm_provider,
        ContainerOperation::Create,
    )?;
    let handler_params = mapped_create_params(call, &kwargs, deps.logging)?;
    let timeout = resolved_timeout(call.timeout, deps.request_timeout_seconds)?;
    Ok(PreparedCreate::Call(ReadyCreate {
        config,
        litellm_params,
        params: handler_params,
        timeout,
    }))
}

// step 2: the kwargs GenericLiteLLMParams reads, then the object the handler and its client see
fn call_parameters(
    call: &ContainerCreateCall,
) -> Result<(IndexMap<String, Json>, Object), ContainerOpError> {
    let kwargs = params::call_kwargs(
        &call.extra,
        call.litellm_metadata.as_ref(),
        call.litellm_call_id.as_deref(),
    )?;
    let litellm_params = params::request_params(
        &kwargs,
        call.api_key.as_ref().map(ExposeSecret::expose_secret),
        call.api_base.as_deref(),
        call.api_version.as_deref(),
    )?;
    Ok((kwargs, litellm_params))
}

// steps 5 and 6: the requested params, the SUPPORTED_PARAMS filter, then the one logging update
fn mapped_create_params(
    call: &ContainerCreateCall,
    kwargs: &IndexMap<String, Json>,
    logging: &Mutex<LoggingState>,
) -> Result<Object, ContainerOpError> {
    // ast-grep-ignore: no-long-names
    let requested = get_requested_container_create_optional_param(&params::create_request(call));
    // ast-grep-ignore: no-long-names
    let mapped = get_optional_params_container_create(&requested);
    let logged: IndexMap<String, Json> = serde_json::from_value(serde_json::to_value(&mapped)?)?;
    let handler_params: Object = serde_json::from_value(serde_json::to_value(&mapped)?)?;
    log_call(
        logging,
        &LoggedCall {
            kwargs,
            optional_params: &logged,
            call_id: call.litellm_call_id.as_deref(),
            provider: &call.custom_llm_provider,
            call_type: CREATE_CALL_TYPE,
        },
    )?;
    Ok(handler_params)
}
