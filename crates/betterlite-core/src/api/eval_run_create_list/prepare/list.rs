use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::transformation::EvalsRequestContext;
use indexmap::IndexMap;

use super::{Operation, StageInput, json_map, log_call, stage};
use crate::api::eval_run_create_list::EvalRunCreateList;
use crate::api::eval_run_create_list::args::ListRunsArgs;
use crate::api::eval_run_create_list::error::EvalRunCallError;
use crate::api::eval_run_create_list::fields::list_params;
use crate::api::eval_run_create_list::providers::{EvalRunProviders, LIST_FLAG, ListRunsCall};

// steps 2 to 7 of LiteLLM's list_runs: the config, the four parameters, validate_environment, the transform, the log
pub(crate) fn prepare_list<'call, Providers, Transport>(
    caller: &'call EvalRunCreateList<Providers, Transport>,
    request: ListRunsArgs<'_>,
    provider: &str,
    logging: &mut LoggingState,
) -> Result<ListRunsCall<'call>, EvalRunCallError>
where
    Providers: EvalRunProviders,
{
    let input = StageInput {
        operation: Operation::List,
        flag: LIST_FLAG,
        extra_headers: request.extra_headers,
    };
    let staged = stage(&caller.providers, provider, request.kwargs, input)?;
    let context = EvalsRequestContext {
        litellm_params: &staged.params,
        headers: &staged.headers,
    };
    let typed = list_params(request)?;
    let (url, query) =
        staged
            .config
            .transform_list_runs_request(request.eval_id, &typed, &context)?;
    let mut optional_params = IndexMap::new();
    optional_params.insert("eval_id".to_owned(), Json::Text(request.eval_id.to_owned()));
    optional_params.extend(json_map(&query)?);
    log_call(
        logging,
        request.kwargs,
        LIST_FLAG,
        optional_params,
        provider,
    )?;
    Ok(ListRunsCall {
        config: staged.config,
        provider: provider.to_owned(),
        url,
        headers: staged.headers,
        query,
        timeout: request.timeout.unwrap_or(caller.request_timeout),
        mode: staged.mode,
    })
}
