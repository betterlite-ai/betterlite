use std::collections::BTreeMap;

use betterlite_models::resolution::ResolutionContext;

use crate::aws_endpoints::provider::{BedrockInvokeProvider, invoke_provider};
use crate::s3_access::models::strip_llm_routing_prefix;

// litellm.model_alias_map, then the record model unless only the target model names a provider.
pub fn resolve_record_model_and_provider<'source>(
    record_model: &'source str,
    target_model: &'source str,
    aliases: &'source BTreeMap<String, String>,
    resolution: &ResolutionContext<'_>,
) -> (&'source str, Option<BedrockInvokeProvider>) {
    let resolved = aliases
        .get(record_model)
        .map_or(record_model, String::as_str);
    let provider = invoke_provider(&strip_llm_routing_prefix(resolved, resolution));
    if provider.is_some() || target_model.is_empty() {
        return (resolved, provider);
    }
    let target_provider = invoke_provider(&strip_llm_routing_prefix(target_model, resolution));
    if target_provider.is_none() {
        return (resolved, provider);
    }
    (target_model, target_provider)
}
