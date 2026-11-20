use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::api::container_contracts::records::ContainerListResponse;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::ExposeSecret;

use super::super::error::{ContainerOpError, ContainerOpFailure, failure};
use super::super::handler::{HandlerCallBase, ListHandlerCall};
use super::super::logging::{LoggedCall, log_call};
use super::super::params;
use super::super::{ContainerCallDeps, ContainerListCall, LIST_CALL_TYPE};
use super::{ContainerOperation, mock_answer, provider_config, resolved_timeout};
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;
// ast-grep-ignore: no-long-names
use crate::containers::container_request_parameters::get_requested_container_list_optional_param;

// a mock answer, or everything the list handler call needs
pub(crate) enum PreparedList<'deps> {
    Mock(ContainerListResponse),
    Call(ReadyList<'deps>),
}

pub(crate) struct ReadyList<'deps> {
    config: &'deps dyn BaseContainerConfig,
    litellm_params: Object,
    extra_query: Option<Object>,
    timeout: Duration,
}

impl ReadyList<'_> {
    pub(crate) const fn handler_call<'call>(
        &'call self,
        call: &'call ContainerListCall,
        logging: &'call Mutex<LoggingState>,
    ) -> ListHandlerCall<'call> {
        ListHandlerCall {
            base: HandlerCallBase {
                config: self.config,
                litellm_params: &self.litellm_params,
                logging,
                extra_headers: call.extra_headers.as_ref(),
                timeout: Some(self.timeout),
            },
            after: call.after.as_deref(),
            limit: call.limit,
            order: call.order.as_deref(),
            extra_query: self.extra_query.as_ref(),
        }
    }
}

// list_containers' steps 1 to 6, shared by the blocking and async operations
pub(crate) fn prepare_list<'deps, Configs>(
    configs: &'deps Configs,
    deps: &ContainerCallDeps<'_>,
    call: &ContainerListCall,
) -> Result<PreparedList<'deps>, ContainerOpFailure>
where
    Configs: ContainerConfigLookup,
{
    let mock = mock_answer(
        call.mock_response.as_ref(),
        "a container list",
        &call.custom_llm_provider,
        deps.redactor,
    )?;
    if let Some(response) = mock {
        return Ok(PreparedList::Mock(response));
    }
    build_list(configs, deps, call)
        .map_err(|error| failure(error, &call.custom_llm_provider, deps.redactor))
}

// steps 2 to 6, in LiteLLM's order, each returning this module's error for the one mapping above
fn build_list<'deps, Configs>(
    configs: &'deps Configs,
    deps: &ContainerCallDeps<'_>,
    call: &ContainerListCall,
) -> Result<PreparedList<'deps>, ContainerOpError>
where
    Configs: ContainerConfigLookup,
{
    let (kwargs, litellm_params) = call_parameters(call)?;
    let config = provider_config(configs, &call.custom_llm_provider, ContainerOperation::List)?;
    log_list_params(call, &kwargs, deps.logging)?;
    let extra_query = params::query_object(call.extra_query.as_ref());
    let timeout = resolved_timeout(call.timeout, deps.request_timeout_seconds)?;
    Ok(PreparedList::Call(ReadyList {
        config,
        litellm_params,
        extra_query,
        timeout,
    }))
}

// step 2: the kwargs GenericLiteLLMParams reads, then the object the handler and its client see
fn call_parameters(
    call: &ContainerListCall,
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

// step 5: the requested params as they were passed, before the logging update records them
fn log_list_params(
    call: &ContainerListCall,
    kwargs: &IndexMap<String, Json>,
    logging: &Mutex<LoggingState>,
) -> Result<(), ContainerOpError> {
    // ast-grep-ignore: no-long-names
    let requested = get_requested_container_list_optional_param(&params::list_request(call));
    let logged: IndexMap<String, Json> = serde_json::from_value(serde_json::to_value(&requested)?)?;
    log_call(
        logging,
        &LoggedCall {
            kwargs,
            optional_params: &logged,
            call_id: call.litellm_call_id.as_deref(),
            provider: &call.custom_llm_provider,
            call_type: LIST_CALL_TYPE,
        },
    )?;
    Ok(())
}
