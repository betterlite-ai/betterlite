use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::evals::transformation::EvalsRequestContext;

use super::{Operation, StageInput, json_map, log_call, stage};
use crate::api::eval_run_create_list::args::CreateRunArgs;
use crate::api::eval_run_create_list::error::EvalRunCallError;
use crate::api::eval_run_create_list::fields::create_request;
use crate::api::eval_run_create_list::providers::{CREATE_FLAG, CreateRunCall, EvalRunProviders};
use crate::api::eval_run_create_list::{CREATE_TIMEOUT, EvalRunCreateList};

// steps 2 to 7 of LiteLLM's create_run: the config, validate_environment, the transform, the logging update
pub(crate) fn prepare_create<'call, Providers, Transport>(
    caller: &'call EvalRunCreateList<Providers, Transport>,
    request: CreateRunArgs<'_>,
    provider: &str,
    logging: &mut LoggingState,
) -> Result<CreateRunCall<'call>, EvalRunCallError>
where
    Providers: EvalRunProviders,
{
    let input = StageInput {
        operation: Operation::Create,
        flag: CREATE_FLAG,
        extra_headers: request.extra_headers,
    };
    let staged = stage(&caller.providers, provider, request.kwargs, input)?;
    let context = EvalsRequestContext {
        litellm_params: &staged.params,
        headers: &staged.headers,
    };
    let typed = create_request(request)?;
    let (url, body) =
        staged
            .config
            .transform_create_run_request(request.eval_id, &typed, &context)?;
    let optional_params = json_map(&body)?;
    log_call(
        logging,
        request.kwargs,
        CREATE_FLAG,
        optional_params,
        provider,
    )?;
    Ok(CreateRunCall {
        config: staged.config,
        provider: provider.to_owned(),
        url,
        headers: staged.headers,
        body,
        timeout: request.timeout.unwrap_or(CREATE_TIMEOUT),
        mode: staged.mode,
    })
}
