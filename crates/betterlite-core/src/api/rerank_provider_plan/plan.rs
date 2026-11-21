use super::call::{GenericParams, RerankCall};
use super::dispatch::{Headers, Params, RerankOperations, SendRequest};
use betterlite_config::api::rerank_provider_plan::RerankSettings;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use stages::{credentials, log, map_options, prepare};
mod stages;
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
    Identity: RerankIdentity + Sync,
    Operations: RerankOperations + Sync,
{
    let prepared = prepare(plan.identity, plan.redactor, &mut call)?;
    let mut mapped = map_options(
        plan.settings,
        plan.operations,
        plan.redactor,
        &prepared,
        &call,
    )?;
    log(plan.redactor, logging, &prepared, &mapped)?;
    let auth = credentials(plan.settings, plan.environment, &prepared, &mut mapped)
        .map_err(|failure| raise(plan.redactor, *failure))?;
    let request = SendRequest {
        config: mapped.config,
        provider: &prepared.provider,
        model: &prepared.resolved.model,
        query: &call.query,
        documents: &call.documents,
        optional_params: &mapped.params,
        litellm_params: &mapped.litellm_params,
        generic_params: &prepared.generic_params,
        api_key: auth.api_key.as_ref(),
        api_base: auth.api_base.as_deref(),
        headers: &auth.headers,
        timeout_seconds: mapped.timeout_seconds,
        is_async: prepared.is_async,
        top_n: call.top_n,
        rank_fields: call.rank_fields.as_deref(),
        return_documents: call.return_documents,
        max_chunks_per_doc: call.max_chunks_per_doc,
        max_tokens_per_doc: call.max_tokens_per_doc,
        instruction: prepared.generic.instruction.as_deref(),
    };
    plan.operations
        .send(request, logging)
        .await
        .map_err(|failure| raise(plan.redactor, *failure))
}
// The error diagnostic runs before B011 maps the failure, as the except block does
fn raise(redactor: &Redactor, failure: RawException) -> RerankException {
    let text = redactor.redact_string(&failure.error_str);
    tracing::error!("{}{}", DIAGNOSTIC_PREFIX, text);
    RerankException {
        class: dispatch(&DispatchInput::Raw(Box::new(failure)), redactor),
        message: text,
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
    for (key, value) in kwargs {
        params.insert(key.clone(), value.clone());
    }
    params
}
