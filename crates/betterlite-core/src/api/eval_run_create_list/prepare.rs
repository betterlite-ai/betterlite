pub(crate) mod create;
pub(crate) mod list;

use std::str::FromStr as _;

use betterlite_context::resource_config_selection::skills_evals::select_evals_config;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_base_config::{Headers, Object};
use indexmap::IndexMap;

use super::error::EvalRunCallError;
use super::providers::{EvalRunProviders, RunMode};
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
    providers
        .evals_config(tag)
        .ok_or_else(|| operation.unsupported(provider))
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

// what a call hands the shared steps: which operation's wording, its kwargs flag and the caller's headers
#[derive(Clone, Copy)]
struct StageInput<'input> {
    operation: Operation,
    flag: &'input str,
    extra_headers: Option<&'input Headers>,
}

// the steps between kwargs and the transform: the flag, the selector, the params and the resolved headers
struct Staged<'call> {
    config: EvalConfig<'call>,
    params: Object,
    headers: Headers,
    mode: RunMode,
}

fn stage<'call, Providers>(
    providers: &'call Providers,
    provider: &str,
    kwargs: &IndexMap<String, Json>,
    input: StageInput<'_>,
) -> Result<Staged<'call>, EvalRunCallError>
where
    Providers: EvalRunProviders,
{
    let mode = RunMode::of(input.flag, kwargs);
    let config = select(providers, provider, input.operation)?;
    let params = litellm_params(&without_flag(kwargs, input.flag))?;
    let headers = config.validate_environment(
        input.extra_headers.cloned().unwrap_or_default(),
        Some(&params),
    )?;
    Ok(Staged {
        config,
        params,
        headers,
        mode,
    })
}
