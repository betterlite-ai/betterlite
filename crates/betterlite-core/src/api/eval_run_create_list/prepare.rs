use std::str::FromStr as _;

use betterlite_context::resource_config_selection::skills_evals::select_evals_config;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::evals::transformation::EvalsRequestContext;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

use super::args::{CreateRunArgs, ListRunsArgs};
use super::error::EvalRunCallError;
use super::fields::{create_request, list_params};
use super::providers::{
    CREATE_FLAG, CreateRunCall, EvalRunProviders, LIST_FLAG, ListRunsCall, RunMode,
};
use super::{CREATE_TIMEOUT, EvalRunCreateList};
use crate::api::eval_http::exchange::EvalConfig;

// which of LiteLLM's two ValueErrors a missing provider config raises
#[derive(Clone, Copy)]
enum Operation {
    Create,
    List,
}

impl Operation {
    // the ValueError ProviderConfigManager's None becomes, named after the operation
    fn unsupported(self, provider: &str) -> EvalRunCallError {
        match self {
            Self::Create => EvalRunCallError::UnsupportedCreate {
                provider: provider.to_owned(),
            },
            Self::List => EvalRunCallError::UnsupportedList {
                provider: provider.to_owned(),
            },
        }
    }
}

// litellm.LlmProviders(custom_llm_provider), then B157's factory for the tag, then the config the caller installed
fn select<'call, Providers>(
    providers: &'call Providers,
    provider: &str,
    operation: Operation,
) -> Result<EvalConfig<'call>, EvalRunCallError>
where
    Providers: EvalRunProviders,
{
    let provider_id =
        ProviderId::from_str(provider).map_err(|source| EvalRunCallError::UnknownProvider {
            provider: provider.to_owned(),
            source,
        })?;
    let Some(tag) = select_evals_config(provider_id) else {
        return Err(operation.unsupported(provider));
    };
    match providers.evals_config(tag) {
        Some(config) => Ok(config),
        None => Err(operation.unsupported(provider)),
    }
}

// create_run and list_runs pop their async flag before anything else reads kwargs
fn without_flag(kwargs: &IndexMap<String, Json>, flag: &str) -> IndexMap<String, Json> {
    let mut forwarded = kwargs.clone();
    forwarded.shift_remove(flag);
    forwarded
}

// GenericLiteLLMParams(**kwargs) as the map the provider config's validate and transform read
fn litellm_params(kwargs: &IndexMap<String, Json>) -> Result<Object, EvalRunCallError> {
    let encoded =
        serde_json::to_value(kwargs).map_err(|source| EvalRunCallError::Params { source })?;
    serde_json::from_value(encoded).map_err(|source| EvalRunCallError::Params { source })
}

// the Object a transform built, as the optional_params the logging updater takes
fn json_map(object: &Object) -> Result<IndexMap<String, Json>, EvalRunCallError> {
    let encoded =
        serde_json::to_value(object).map_err(|source| EvalRunCallError::Params { source })?;
    serde_json::from_value(encoded).map_err(|source| EvalRunCallError::Params { source })
}

// LiteLLM's update_from_kwargs: the flag is gone from kwargs, and litellm_params carries only the call id
fn log_call(
    logging: &mut LoggingState,
    kwargs: &IndexMap<String, Json>,
    flag: &str,
    optional_params: IndexMap<String, Json>,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let mut forwarded = kwargs.clone();
    forwarded.shift_remove(flag);
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        forwarded
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    update_from_kwargs(
        logging,
        &forwarded,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}

impl<Providers, Transport> EvalRunCreateList<Providers, Transport>
where
    Providers: EvalRunProviders,
{
    // steps 2 to 7 of LiteLLM's create_run: the config, validate_environment, the transform, the logging update
    pub(super) fn prepare_create<'call>(
        &'call self,
        request: CreateRunArgs<'_>,
        provider: &str,
        logging: &mut LoggingState,
    ) -> Result<CreateRunCall<'call>, EvalRunCallError> {
        let mode = RunMode::of(CREATE_FLAG, request.kwargs);
        let config = select(&self.providers, provider, Operation::Create)?;
        let forwarded = without_flag(request.kwargs, CREATE_FLAG);
        let params = litellm_params(&forwarded)?;
        let headers = config.validate_environment(
            request.extra_headers.cloned().unwrap_or_default(),
            Some(&params),
        )?;
        let context = EvalsRequestContext {
            litellm_params: &params,
            headers: &headers,
        };
        let typed = create_request(request)?;
        let (url, body) = config.transform_create_run_request(request.eval_id, &typed, &context)?;
        let optional_params = json_map(&body)?;
        log_call(
            logging,
            request.kwargs,
            CREATE_FLAG,
            optional_params,
            provider,
        )?;
        Ok(CreateRunCall {
            config,
            provider: provider.to_owned(),
            url,
            headers,
            body,
            timeout: request.timeout.unwrap_or(CREATE_TIMEOUT),
            mode,
        })
    }

    // steps 2 to 7 of LiteLLM's list_runs: the config, the four parameters, validate_environment, the transform, the log
    pub(super) fn prepare_list<'call>(
        &'call self,
        request: ListRunsArgs<'_>,
        provider: &str,
        logging: &mut LoggingState,
    ) -> Result<ListRunsCall<'call>, EvalRunCallError> {
        let mode = RunMode::of(LIST_FLAG, request.kwargs);
        let config = select(&self.providers, provider, Operation::List)?;
        let params = litellm_params(&without_flag(request.kwargs, LIST_FLAG))?;
        let headers = config.validate_environment(
            request.extra_headers.cloned().unwrap_or_default(),
            Some(&params),
        )?;
        let context = EvalsRequestContext {
            litellm_params: &params,
            headers: &headers,
        };
        let typed = list_params(request)?;
        let (url, query) = config.transform_list_runs_request(request.eval_id, &typed, &context)?;
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
            config,
            provider: provider.to_owned(),
            url,
            headers,
            query,
            timeout: request.timeout.unwrap_or(self.request_timeout),
            mode,
        })
    }
}
