use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};

const BEDROCK_ROUTING_PREFIX: &str = "bedrock/";
const INVOKE_ROUTE_PREFIX: &str = "invoke/";

#[must_use]
pub fn bedrock_model_id(model: &str) -> String {
    let Some(stripped) = model.strip_prefix(BEDROCK_ROUTING_PREFIX) else {
        return model.to_owned();
    };
    stripped.to_owned()
}

#[must_use]
pub fn strip_llm_routing_prefix(model: &str, context: &ResolutionContext<'_>) -> String {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    match resolve(&request, context) {
        Ok(resolved) => resolved.model,
        Err(source) => {
            tracing::error!(
                error = %source,
                "litellm.llms.bedrock.files.transformation.py::_strip_llm_routing_prefix() - Error inferring custom_llm_provider"
            );
            model.to_owned()
        }
    }
}

#[must_use]
pub fn invoke_route_model(model: &str, context: &ResolutionContext<'_>) -> String {
    let stripped = strip_llm_routing_prefix(model, context);
    let rest = stripped
        .strip_prefix(INVOKE_ROUTE_PREFIX)
        .unwrap_or(&stripped);
    format!("{INVOKE_ROUTE_PREFIX}{rest}")
}
