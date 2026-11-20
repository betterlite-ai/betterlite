use betterlite_config::api::rerank_provider_plan::RerankSettings;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::branches::{BranchInput, select_credentials};
use super::call::{GenericParams, RerankCall, TimeoutArg};
use super::dispatch::{
    Headers, OptionsRequest, Params, RerankOperations, SendRequest, value_failure,
};
use super::selection::selected_config;

// LiteLLM's `verbose_logger.error("Error in rerank: %s", e)`, the line every failed call starts with
pub const DIAGNOSTIC_PREFIX: &str = "Error in rerank: ";

#[derive(Debug, Clone)]
pub struct RerankException {
    pub class: ErrorClass,
    pub message: String,
}

pub trait RerankIdentity {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError>;
}

pub struct CatalogIdentity<'context, 'borrow> {
    pub context: &'borrow ResolutionContext<'context>,
}

impl RerankIdentity for CatalogIdentity<'_, '_> {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError> {
        resolve(request, self.context)
    }
}

pub struct RerankPlan<'plan, Identity, Operations> {
    pub identity: &'plan Identity,
    pub operations: &'plan Operations,
    pub settings: &'plan RerankSettings,
    pub environment: &'plan dyn EnvironmentLookup,
    pub redactor: &'plan Redactor,
}

pub async fn rerank<Identity, Operations>(
    plan: &RerankPlan<'_, Identity, Operations>,
    mut call: RerankCall,
    logging: &mut LoggingState,
) -> Result<RerankResponse, RerankException>
where
    Identity: RerankIdentity,
    Operations: RerankOperations,
{
    let mut kwargs = std::mem::take(&mut call.kwargs);
    // `kwargs.pop("arerank", False) is True`: only the boolean true selects the async transport
    let is_async = matches!(kwargs.shift_remove("arerank"), Some(Json::Bool(true)));
    let provider_hint = call.custom_llm_provider.clone().unwrap_or_default();
    let generic = GenericParams::read(&kwargs).map_err(|error| {
        raise(
            plan.redactor,
            value_failure(&error.to_string(), &call.model, &provider_hint),
        )
    })?;
    let resolution_request = ResolutionRequest {
        model: &call.model,
        custom_llm_provider: call.custom_llm_provider.as_deref(),
        api_base: generic.api_base.as_deref(),
        api_key: generic.api_key.as_ref(),
    };
    let resolved = plan
        .identity
        .resolve(&resolution_request)
        .map_err(|error| {
            raise(
                plan.redactor,
                value_failure(&error.to_string(), &call.model, &provider_hint),
            )
        })?;
    let provider = resolved.provider.slug().to_owned();
    let config = selected_config(
        &resolved.provider,
        &resolved.model,
        generic.api_base.as_deref(),
        &present_version_params(call.max_chunks_per_doc, call.max_tokens_per_doc),
    );
    // `kwargs.get("drop_params") or litellm.drop_params or False`: a call false cannot switch the global true off
    let drop_params = generic.drop_params.unwrap_or(false) || plan.settings.drop_params;
    let mut non_default_params = params_from_kwargs(&kwargs);
    let mut optional_params = plan
        .operations
        .map_options(OptionsRequest {
            config,
            model: &resolved.model,
            drop_params,
            query: &call.query,
            documents: &call.documents,
            custom_llm_provider: Some(provider.as_str()),
            top_n: call.top_n,
            rank_fields: call.rank_fields.as_deref(),
            return_documents: call.return_documents,
            max_chunks_per_doc: call.max_chunks_per_doc,
            max_tokens_per_doc: call.max_tokens_per_doc,
            instruction: generic.instruction.as_deref(),
            non_default_params: &mut non_default_params,
        })
        .map_err(|failure| raise(plan.redactor, failure))?;
    // a string timeout is converted with float(...) only after the provider mapped the options
    let timeout_seconds = generic
        .timeout
        .as_ref()
        .map(TimeoutArg::seconds)
        .transpose()
        .map_err(|error| {
            raise(
                plan.redactor,
                value_failure(&error.to_string(), &resolved.model, &provider),
            )
        })?;
    let litellm_params = rerank_litellm_params(&kwargs);
    let headers = call_headers(&generic, plan.settings);
    log_call(
        logging,
        LogCall {
            kwargs: &kwargs,
            model: &resolved.model,
            user: generic.user.as_deref(),
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            provider: &provider,
        },
    )
    .map_err(|error| {
        raise(
            plan.redactor,
            value_failure(&error.to_string(), &resolved.model, &provider),
        )
    })?;
    let generic_params = params_from_kwargs(&kwargs);
    let credentials = select_credentials(
        BranchInput {
            provider: &provider,
            model: &resolved.model,
            config,
            resolved: &resolved,
            generic: &generic,
            generic_params: &generic_params,
            settings: plan.settings,
            environment: plan.environment,
            headers,
        },
        &mut optional_params,
    )
    .map_err(|failure| raise(plan.redactor, failure))?;
    let request = SendRequest {
        config,
        provider: &provider,
        model: &resolved.model,
        query: &call.query,
        documents: &call.documents,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        generic_params: &generic_params,
        api_key: credentials.api_key.as_ref(),
        api_base: credentials.api_base.as_deref(),
        headers: &credentials.headers,
        timeout_seconds,
        is_async,
        top_n: call.top_n,
        rank_fields: call.rank_fields.as_deref(),
        return_documents: call.return_documents,
        max_chunks_per_doc: call.max_chunks_per_doc,
        max_tokens_per_doc: call.max_tokens_per_doc,
        instruction: generic.instruction.as_deref(),
    };
    plan.operations
        .send(request, logging)
        .await
        .map_err(|failure| raise(plan.redactor, failure))
}

// The error diagnostic runs before B011 maps the failure, as the except block does
fn raise(redactor: &Redactor, failure: RawException) -> RerankException {
    tracing::error!("{}{}", DIAGNOSTIC_PREFIX, failure.error_str);
    RerankException {
        class: dispatch(&DispatchInput::Raw(Box::new(failure)), redactor),
        message: failure.error_str,
    }
}

fn present_version_params(max_chunks: Option<i64>, max_tokens: Option<i64>) -> Vec<String> {
    [
        ("max_chunks_per_doc", max_chunks),
        ("max_tokens_per_doc", max_tokens),
    ]
    .into_iter()
    .filter(|(_, value)| value.is_some())
    .map(|(name, _)| String::from(name))
    .collect()
}

fn call_headers(generic: &GenericParams, settings: &RerankSettings) -> Headers {
    if generic.headers.is_empty() {
        return settings.headers.clone().unwrap_or_default();
    }
    generic.headers.clone()
}

fn params_from_kwargs(kwargs: &IndexMap<String, Json>) -> Params {
    kwargs
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// `{litellm_call_id, proxy_server_request, model_info, preset_cache_key: None, stream_response: {}, **optional_params.model_dump(exclude_unset=True)}`
fn rerank_litellm_params(kwargs: &IndexMap<String, Json>) -> Params {
    let mut params = Params::new();
    for name in [
        "litellm_call_id",
        "proxy_server_request",
        "model_info",
        "preset_cache_key",
    ] {
        params.insert(
            name.to_owned(),
            kwargs.get(name).cloned().unwrap_or(Json::Null),
        );
    }
    params.insert(
        "stream_response".to_owned(),
        kwargs
            .get("stream_response")
            .cloned()
            .unwrap_or_else(|| Json::Object(IndexMap::new())),
    );
    for (name, value) in kwargs {
        params.insert(name.clone(), value.clone());
    }
    params
}

struct LogCall<'request> {
    kwargs: &'request IndexMap<String, Json>,
    model: &'request str,
    user: Option<&'request str>,
    optional_params: &'request Params,
    litellm_params: &'request Params,
    provider: &'request str,
}

fn log_call(logging: &mut LoggingState, call: LogCall<'_>) -> Result<(), LoggingStateError> {
    let mut additional = IndexMap::new();
    additional.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(call.provider.to_owned()),
    );
    let litellm = as_index_map(call.litellm_params);
    update_from_kwargs(
        logging,
        call.kwargs,
        Some(&litellm),
        CallUpdate {
            model: Some(call.model.to_owned()),
            user: call.user.map(str::to_owned),
            optional_params: as_index_map(call.optional_params),
            additional_params: additional,
        },
    )
}

fn as_index_map(params: &Params) -> IndexMap<String, Json> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
