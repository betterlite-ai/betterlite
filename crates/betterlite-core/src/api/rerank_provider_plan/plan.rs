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

// What a failed call raises: the class B011 mapped and the text the diagnostic logged
#[derive(Debug, Clone)]
pub struct RerankException {
    pub class: ErrorClass,
    pub message: String,
}

// get_llm_provider, which the dispatcher uses before it decides anything else
pub trait RerankIdentity {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError>;
}

// With no policy of its own, the dispatcher asks B174 to resolve the call's identity
pub struct CatalogIdentity<'context, 'borrow> {
    pub context: &'borrow ResolutionContext<'context>,
}

impl RerankIdentity for CatalogIdentity<'_, '_> {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError> {
        resolve(request, self.context)
    }
}

// Everything the dispatcher reads besides the call: the identity seam, the provider seams and the settings
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
    let generic = match GenericParams::read(&kwargs) {
        Ok(generic) => generic,
        Err(error) => {
            return Err(raise(
                plan.redactor,
                value_failure(&error.to_string(), &call.model, &provider_hint),
            ));
        }
    };
    let resolution_request = ResolutionRequest {
        model: &call.model,
        custom_llm_provider: call.custom_llm_provider.as_deref(),
        api_base: generic.api_base.as_deref(),
        api_key: generic.api_key.as_ref(),
    };
    let resolved = match plan.identity.resolve(&resolution_request) {
        Ok(resolved) => resolved,
        Err(error) => {
            return Err(raise(
                plan.redactor,
                value_failure(&error.to_string(), &call.model, &provider_hint),
            ));
        }
    };
    let provider = resolved.provider.slug().to_owned();
    let config = selected_config(
        &resolved.provider,
        &resolved.model,
        generic.api_base.as_deref(),
        &present_version_params(call.max_chunks_per_doc, call.max_tokens_per_doc),
    );
    let drop_params = generic.drop_params.unwrap_or(plan.settings.drop_params);
    let mut non_default_params = params_from_kwargs(&kwargs);
    let mut optional_params = match plan.operations.map_options(OptionsRequest {
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
    }) {
        Ok(params) => params,
        Err(failure) => return Err(raise(plan.redactor, failure)),
    };
    // a string timeout is converted with float(...) only after the provider mapped the options
    let timeout_seconds = match generic
        .timeout
        .as_ref()
        .map(TimeoutArg::seconds)
        .transpose()
    {
        Ok(seconds) => seconds,
        Err(error) => {
            return Err(raise(
                plan.redactor,
                value_failure(&error.to_string(), &resolved.model, &provider),
            ));
        }
    };
    let litellm_params = rerank_litellm_params(&kwargs);
    let headers = call_headers(&generic, plan.settings);
    if let Err(error) = log_call(
        logging,
        &kwargs,
        &resolved.model,
        generic.user.as_deref(),
        &optional_params,
        &litellm_params,
        &provider,
    ) {
        return Err(raise(
            plan.redactor,
            value_failure(&error.to_string(), &resolved.model, &provider),
        ));
    }
    let generic_params = params_from_kwargs(&kwargs);
    let credentials = match select_credentials(
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
    ) {
        Ok(credentials) => credentials,
        Err(failure) => return Err(raise(plan.redactor, failure)),
    };
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
    match plan.operations.send(request, logging).await {
        Ok(response) => Ok(response),
        Err(failure) => Err(raise(plan.redactor, failure)),
    }
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
    let mut present = Vec::new();
    if max_chunks.is_some() {
        present.push(String::from("max_chunks_per_doc"));
    }
    if max_tokens.is_some() {
        present.push(String::from("max_tokens_per_doc"));
    }
    present
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

fn log_call(
    logging: &mut LoggingState,
    kwargs: &IndexMap<String, Json>,
    model: &str,
    user: Option<&str>,
    optional_params: &Params,
    litellm_params: &Params,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let logged_optional: IndexMap<String, Json> = optional_params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let logged_litellm: IndexMap<String, Json> = litellm_params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let mut additional = IndexMap::new();
    additional.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    update_from_kwargs(
        logging,
        kwargs,
        Some(&logged_litellm),
        CallUpdate {
            model: Some(model.to_owned()),
            user: user.map(str::to_owned),
            optional_params: logged_optional,
            additional_params: additional,
        },
    )
}
