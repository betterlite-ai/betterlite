use super::ParameterMap;
use super::prefixes::strip_responses_routing_prefix;
use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest, ResolvedModel};
pub trait ModelResolver {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError>;
}
pub fn resolve_model_provider(
    resolver: &dyn ModelResolver,
    model: &str,
    custom_llm_provider: Option<&str>,
    params: &mut CallParams,
    local_vars: &mut ParameterMap,
) -> Result<(String, String), ResolutionError> {
    if custom_llm_provider.is_some()
        && params
            .custom_llm_provider
            .as_deref()
            .is_none_or(str::is_empty)
    {
        params.custom_llm_provider = custom_llm_provider.map(str::to_owned);
    }
    let resolved = resolver.resolve_model(&ResolutionRequest {
        model,
        custom_llm_provider: params.custom_llm_provider.as_deref(),
        api_base: params.api_base.as_deref(),
        api_key: params.api_key.as_ref(),
    })?;
    let provider = resolved.provider.slug().to_owned();
    local_vars.insert(
        String::from("custom_llm_provider"),
        JsonValue::Text(provider.clone()),
    );
    if let Some(key) = resolved.api_key {
        params.api_key = Some(key);
    }
    if let Some(base) = resolved.api_base {
        params.api_base = Some(base);
    }
    Ok((strip_responses_routing_prefix(&resolved.model), provider))
}
