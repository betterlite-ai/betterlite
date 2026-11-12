use super::JsonObject;
use crate::catalog_generalizations::CatalogGeneralizations;
use crate::decisions::Json;
use crate::identity::provider_id::ProviderId;
use crate::model_catalog_keys::strip_model_name;

const BEDROCK_REGION_PREFIXES: [&str; 8] = [
    "us.",
    "eu.",
    "apac.",
    "jp.",
    "au.",
    "us-gov.",
    "global.",
    "ap-northeast-1.",
];

pub const CACHE_PRICING_FIELDS: [&str; 5] = [
    "cache_creation_input_token_cost",
    "cache_creation_input_token_cost_above_1hr",
    "cache_creation_input_token_cost_above_200k_tokens",
    "cache_read_input_token_cost",
    "cache_read_input_token_cost_above_200k_tokens",
];

fn candidate_names(
    key: &str,
    provider: &str,
    bedrock_base_model: fn(&str) -> String,
) -> Vec<String> {
    let mut candidates = Vec::new();
    let segments: Vec<&str> = key.split('/').collect();
    let mut index = 0_usize;
    while index < segments.len().saturating_sub(1)
        && segments
            .get(index)
            .is_some_and(|segment| segment.parse::<ProviderId>().is_ok())
    {
        index = index.saturating_add(1);
        candidates.push(segments.get(index..).unwrap_or_default().join("/"));
    }
    let base = candidates.last().map_or(key, String::as_str).to_owned();
    for prefix in BEDROCK_REGION_PREFIXES {
        if let Some(rest) = base.strip_prefix(prefix) {
            candidates.push(rest.to_owned());
        }
    }
    if !provider.is_empty() {
        let stripped = strip_model_name(&base, Some(provider), bedrock_base_model);
        if stripped != base {
            candidates.push(stripped);
        }
    }
    candidates
}

#[must_use]
pub fn builtin_cost_entry(
    model_cost: &JsonObject,
    key: &str,
    provider: &str,
    bedrock_base_model: fn(&str) -> String,
) -> Option<JsonObject> {
    candidate_names(key, provider, bedrock_base_model)
        .iter()
        .find_map(|candidate| match model_cost.get(candidate) {
            Some(Json::Object(entry))
                if entry
                    .get("litellm_provider")
                    .is_some_and(|declared| !matches!(declared, Json::Null)) =>
            {
                Some(entry.clone())
            }
            _ => None,
        })
}

#[must_use]
pub fn is_generalized_model_info(
    model_cost: &JsonObject,
    model_info: &JsonObject,
    generalizations: &CatalogGeneralizations,
) -> bool {
    let Some(Json::Text(key)) = model_info.get("key") else {
        return false;
    };
    !model_cost.contains_key(key) && generalizations.match_capabilities(key).is_some()
}
