use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::{BaseEvalsAPIConfig, EvalsRequestContext};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::error::{EvalLifecycleError, EvalLifecycleFailure, failure};
use super::{
    CANCEL_ASYNC_FLAG, DEFAULT_OPENAI_API_BASE, DEFAULT_PROVIDER, DELETE_ASYNC_FLAG,
    EvalLifecycleCall, EvalLifecycleConfigLookup, EvalLifecycleDeps, LITELLM_CALL_ID,
};

// What the handler call needs: the provider config, the URL and headers the transform built, and `timeout or request_timeout`
pub(crate) struct Prepared<Config> {
    pub(crate) provider: String,
    pub(crate) config: Config,
    pub(crate) url: String,
    pub(crate) headers: Headers,
    pub(crate) timeout: Duration,
}

// The two operations differ in the transform they call and in the keyword their async wrapper leaves
#[derive(Clone, Copy)]
pub(crate) enum EvalAction {
    Delete,
    Cancel,
}

impl EvalAction {
    // the verb LiteLLM's ValueError names
    const fn verb(self) -> &'static str {
        match self {
            Self::Delete => "DELETE",
            Self::Cancel => "CANCEL",
        }
    }

    // the keyword kwargs.pop reads before GenericLiteLLMParams sees the call keywords
    const fn async_flag(self) -> &'static str {
        match self {
            Self::Delete => DELETE_ASYNC_FLAG,
            Self::Cancel => CANCEL_ASYNC_FLAG,
        }
    }
}

// LiteLLM's delete_eval and cancel_eval body, steps 1 to 9 in order
pub(crate) fn prepare<Configs>(
    deps: &EvalLifecycleDeps<'_, Configs>,
    call: &EvalLifecycleCall<'_>,
    action: EvalAction,
    state: &mut LoggingState,
) -> Result<Prepared<Configs::Config>, EvalLifecycleFailure>
where
    Configs: EvalLifecycleConfigLookup + ?Sized,
{
    let provider = call
        .custom_llm_provider
        .unwrap_or(DEFAULT_PROVIDER)
        .to_owned();
    let config = config_for(deps.configs, &provider, action, deps.redactor)?;
    let mut kwargs = call.kwargs.clone();
    kwargs.shift_remove(action.async_flag());
    let params = params_object(&kwargs).map_err(|source| {
        failure(
            EvalLifecycleError::Params { source },
            &provider,
            deps.redactor,
        )
    })?;
    let given = call.extra_headers.cloned().unwrap_or_default();
    let validated = config
        .validate_environment(given, Some(&params))
        .map_err(|error| failure(EvalLifecycleError::Config(error), &provider, deps.redactor))?;
    let context = EvalsRequestContext {
        litellm_params: &params,
        headers: &validated,
    };
    let (url, headers) = request_target(&config, action, call.eval_id, &params, &context)
        .map_err(|error| failure(EvalLifecycleError::Config(error), &provider, deps.redactor))?;
    log_call(state, &kwargs, call.eval_id, &provider)
        .map_err(|error| failure(EvalLifecycleError::Logging(error), &provider, deps.redactor))?;
    Ok(Prepared {
        provider,
        config,
        url,
        headers,
        timeout: timeout(call.timeout, deps.request_timeout),
    })
}

// step 3: ProviderConfigManager.get_provider_evals_api_config, None for a provider with no evals API
fn config_for<Configs>(
    configs: &Configs,
    provider: &str,
    action: EvalAction,
    redactor: &Redactor,
) -> Result<Configs::Config, EvalLifecycleFailure>
where
    Configs: EvalLifecycleConfigLookup + ?Sized,
{
    configs.eval_config(provider).ok_or_else(|| {
        let error = EvalLifecycleError::Unsupported {
            operation: action.verb(),
            provider: provider.to_owned(),
        };
        failure(error, provider, redactor)
    })
}

// steps 6 and 7: the transform builds the URL and headers, and a cancel body stays bound here, never handed to the handler
fn request_target(
    config: &(dyn BaseEvalsAPIConfig<Error = EvalsError> + Send + Sync),
    action: EvalAction,
    eval_id: &str,
    params: &Object,
    context: &EvalsRequestContext<'_>,
) -> Result<(String, Headers), EvalsError> {
    let api_base = api_base(params);
    match action {
        EvalAction::Delete => config.transform_delete_eval_request(eval_id, api_base, context),
        EvalAction::Cancel => {
            let (url, headers, _) =
                config.transform_cancel_eval_request(eval_id, api_base, context)?;
            Ok((url, headers))
        }
    }
}

// step 5: litellm_params.api_base or DEFAULT_OPENAI_API_BASE, where an empty string is falsey
fn api_base(params: &Object) -> &str {
    present(params.get("api_base").and_then(|value| value.as_str()))
        .unwrap_or(DEFAULT_OPENAI_API_BASE)
}

// GenericLiteLLMParams reads an empty string as absent, as LiteLLM's `or` does
const fn present(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

// step 4: GenericLiteLLMParams(**kwargs) builds a params object from the keywords left after the async flag is popped
fn params_object(kwargs: &IndexMap<String, Json>) -> Result<Object, serde_json::Error> {
    let mut params = Object::new();
    for (name, value) in kwargs {
        params.insert(name.clone(), serde_json::to_value(value)?);
    }
    Ok(params)
}

// step 8: update_from_kwargs with eval_id, the call id, no model and no user
fn log_call(
    state: &mut LoggingState,
    kwargs: &IndexMap<String, Json>,
    eval_id: &str,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let litellm_params = IndexMap::from([(
        LITELLM_CALL_ID.to_owned(),
        kwargs.get(LITELLM_CALL_ID).cloned().unwrap_or(Json::Null),
    )]);
    let optional_params = IndexMap::from([("eval_id".to_owned(), Json::Text(eval_id.to_owned()))]);
    let additional_params = IndexMap::from([(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    )]);
    update_from_kwargs(
        state,
        kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}

// step 9: `timeout or request_timeout`, where a zero timeout is falsey and takes B198's request timeout
const fn timeout(supplied: Option<Duration>, request_timeout: Duration) -> Duration {
    match supplied {
        Some(value) if !value.is_zero() => value,
        Some(_) | None => request_timeout,
    }
}
