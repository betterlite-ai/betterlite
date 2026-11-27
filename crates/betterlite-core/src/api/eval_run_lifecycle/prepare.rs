use std::str::FromStr as _;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::EvalsRequestContext;
use betterlite_openai::openai_base_config::{Headers, Object};
use indexmap::IndexMap;

use super::error::{EvalRunError, EvalRunFailure, RunAction, failure};
use super::handlers::{EvalRunHandlers, RunHandlerRequest};
use super::{DEFAULT_OPENAI_API_BASE, DEFAULT_PROVIDER, EvalRunDeps, EvalRunRequest};
use crate::api::eval_http::exchange::EvalConfig;

const API_BASE: &str = "api_base";
const LITELLM_CALL_ID: &str = "litellm_call_id";

// One run call after the provider lookup: what B9371's handler takes and what the logging update recorded
pub(crate) struct Prepared<'deps> {
    pub(crate) provider: String,
    pub(crate) config: EvalConfig<'deps>,
    pub(crate) url: String,
    pub(crate) headers: Headers,
    pub(crate) timeout: Duration,
    pub(crate) is_async: bool,
}

impl Prepared<'_> {
    // the arguments B9371's handler takes, all borrowed from this call
    pub(crate) fn handler_request(&self) -> RunHandlerRequest<'_> {
        RunHandlerRequest {
            config: self.config,
            url: &self.url,
            headers: &self.headers,
            timeout: Some(self.timeout),
            is_async: self.is_async,
        }
    }
}

// LiteLLM's run body from the async-flag pop to the pre-call logging update, shared by the three operations
pub(crate) fn prepare<'deps, Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'deps, Handler>,
    action: RunAction,
    request: &EvalRunRequest<'_>,
    logging: &mut LoggingState,
) -> Result<Prepared<'deps>, EvalRunFailure> {
    let caller = request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
    attempt(deps, action, request, request.kwargs.clone(), logging)
        .map_err(|error| failure(error, caller, deps.redactor))
}

// The @client wrapper sets its flag in kwargs, and the sync body pops it before GenericLiteLLMParams reads the rest
pub(crate) fn prepare_async<'deps, Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'deps, Handler>,
    action: RunAction,
    request: &EvalRunRequest<'_>,
    logging: &mut LoggingState,
) -> Result<Prepared<'deps>, EvalRunFailure> {
    let mut kwargs = request.kwargs.clone();
    kwargs.insert(action.async_flag().to_owned(), Json::Bool(true));
    let flagged = EvalRunRequest {
        kwargs: &kwargs,
        ..*request
    };
    prepare(deps, action, &flagged, logging)
}

// the steps that can fail, each error mapped with the provider once it has been defaulted
fn attempt<'deps, Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'deps, Handler>,
    action: RunAction,
    request: &EvalRunRequest<'_>,
    mut kwargs: IndexMap<String, Json>,
    logging: &mut LoggingState,
) -> Result<Prepared<'deps>, EvalRunError> {
    let caller = request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
    let is_async = matches!(
        kwargs.shift_remove(action.async_flag()),
        Some(Json::Bool(true))
    );
    let params = generic_params(&kwargs)?;
    // GenericLiteLLMParams validates api_base's declared type while it is built, before the provider lookup
    let base = api_base(&kwargs)?;
    let config = config_for(deps, action, caller)?;
    let given = request.extra_headers.cloned().unwrap_or_default();
    let headers = config.validate_environment(given, Some(&params))?;
    let context = EvalsRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let (url, forwarded) = transform(action, config, request, base, &context)?;
    log_call(logging, request, &kwargs, caller)?;
    Ok(Prepared {
        provider: caller.to_owned(),
        config,
        url,
        headers: forwarded,
        timeout: request.timeout.unwrap_or(deps.request_timeout),
        is_async,
    })
}

// GenericLiteLLMParams(**kwargs): extra="allow" keeps every keyword, and validate_environment reads api_key back out of it
fn generic_params(kwargs: &IndexMap<String, Json>) -> Result<Object, serde_json::Error> {
    let mut params = Object::new();
    for (name, value) in kwargs {
        params.insert(name.clone(), serde_json::to_value(value)?);
    }
    Ok(params)
}

// litellm.LlmProviders(provider), then ProviderConfigManager.get_provider_evals_api_config, whose only config is OpenAI's
fn config_for<'deps, Handler: EvalRunHandlers + ?Sized>(
    deps: &EvalRunDeps<'deps, Handler>,
    action: RunAction,
    provider: &str,
) -> Result<EvalConfig<'deps>, EvalRunError> {
    let Ok(id) = ProviderId::from_str(provider) else {
        return Err(EvalRunError::UnknownProvider {
            provider: provider.to_owned(),
        });
    };
    if id == ProviderId::Openai {
        return Ok(deps.config);
    }
    Err(EvalRunError::Unsupported {
        action,
        provider: provider.to_owned(),
    })
}

// litellm_params.api_base or DEFAULT_OPENAI_API_BASE: absent or null takes the default, and docs/compatibility.md keeps a given string as it comes
fn api_base(kwargs: &IndexMap<String, Json>) -> Result<&str, EvalRunError> {
    match kwargs.get(API_BASE) {
        None | Some(Json::Null) => Ok(DEFAULT_OPENAI_API_BASE),
        Some(Json::Text(text)) => Ok(text),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(EvalRunError::ApiBase)
        }
    }
}

// transform_get_run_request hands back a URL and headers; cancel and delete also build a body the handler never sends
fn transform(
    action: RunAction,
    config: EvalConfig<'_>,
    request: &EvalRunRequest<'_>,
    base: &str,
    context: &EvalsRequestContext<'_>,
) -> Result<(String, Headers), EvalsError> {
    match action {
        RunAction::Get => {
            config.transform_get_run_request(request.eval_id, request.run_id, base, context)
        }
        RunAction::Cancel => {
            let (url, headers, _body) = config.transform_cancel_run_request(
                request.eval_id,
                request.run_id,
                base,
                context,
            )?;
            Ok((url, headers))
        }
        RunAction::Delete => {
            let (url, headers, _body) = config.transform_delete_run_request(
                request.eval_id,
                request.run_id,
                base,
                context,
            )?;
            Ok((url, headers))
        }
    }
}

// litellm_logging_obj.update_from_kwargs(..., model=None, optional_params={"eval_id", "run_id"}, litellm_params={"litellm_call_id"})
fn log_call(
    logging: &mut LoggingState,
    request: &EvalRunRequest<'_>,
    kwargs: &IndexMap<String, Json>,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let optional_params = IndexMap::from([
        ("eval_id".to_owned(), Json::Text(request.eval_id.to_owned())),
        ("run_id".to_owned(), Json::Text(request.run_id.to_owned())),
    ]);
    let litellm_params = IndexMap::from([(
        LITELLM_CALL_ID.to_owned(),
        kwargs.get(LITELLM_CALL_ID).cloned().unwrap_or(Json::Null),
    )]);
    let additional_params = IndexMap::from([(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    )]);
    update_from_kwargs(
        logging,
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
